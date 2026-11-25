#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Source adapters: turn a file into a typed semantic model.
//!
//! The connectors ask DuckDB to describe a file's schema and map that onto the application's
//! small type set. Types the application cannot classify become [`DataType::Unknown`] rather than
//! an error, so a model can always be built; numeric columns become measures and everything else
//! a dimension, which the user can then override.
//!
//! Discovery is a read-only describe; it never imports data into a store.

use std::path::Path;

use slickbi_engine::arrow::array::{Array, StringArray};
use slickbi_engine::{Engine, SourceFormat};
use slickbi_spec::{
    Column, ColumnId, Connection, ConnectionId, ConnectionKind, DataModel, DataModelId, DataType,
    Field, FieldId, FieldSource, Role, Table, TableId,
};
use thiserror::Error;

mod database;
mod sheet;
mod temporal;

pub use database::{DatabaseKind, DatabaseSource, DatabaseTable, list_tables, read_database};
pub use sheet::{
    SheetTable, SpreadsheetKind, open_spreadsheet, read_spreadsheet, spreadsheet_sheets,
};

/// The name a discovered table registers under.
pub const TABLE_NAME: &str = "data";

/// Opens a file source: registers it as the `data` table and returns its model.
///
/// CSV and TSV sources are registered as a view over the file, so every query reads the file's
/// current contents. Text columns that hold currency-formatted numbers (`$1,234.56`, `-$7.77`,
/// `($1.23)`), which DuckDB would otherwise classify as text, are registered through an expression
/// that strips the formatting, and the model reports them as numbers.
///
/// # Errors
///
/// Returns [`ConnectorError`] if the file cannot be read or registered.
pub fn open_file(
    engine: &Engine,
    path: &Path,
    format: SourceFormat,
) -> Result<DataModel, ConnectorError> {
    match format {
        SourceFormat::Csv => open_csv(engine, path),
        SourceFormat::Parquet | SourceFormat::Json => {
            let model = discover(engine, path, format)?;
            engine.register(TABLE_NAME, path, format)?;
            Ok(model)
        }
    }
}

/// Discovers the schema of a file and builds a single-table model over it.
///
/// This is a raw describe: it reports the types the format itself implies, without the cleaning
/// [`open_file`] applies.
///
/// # Errors
///
/// Returns [`ConnectorError`] if the file cannot be read or described.
pub fn discover(
    engine: &Engine,
    path: &Path,
    format: SourceFormat,
) -> Result<DataModel, ConnectorError> {
    describe_model(engine, &source_sql(path, format))
}

/// The table function that reads a file of this format.
fn source_sql(path: &Path, format: SourceFormat) -> String {
    let escaped = path.to_string_lossy().replace('\'', "''");
    match format {
        SourceFormat::Csv => match slash_date_pattern(path) {
            Some(pattern) => format!("read_csv_auto('{escaped}', dateformat='{pattern}')"),
            None => format!("read_csv_auto('{escaped}')"),
        },
        SourceFormat::Parquet => format!("read_parquet('{escaped}')"),
        SourceFormat::Json => format!("read_json_auto('{escaped}')"),
    }
}

/// A `dateformat` for a CSV whose dates are written with slashes, when one form is unambiguous.
///
/// DuckDB guesses `01/02/25` as year/month/day, which turns the second of January into the
/// twenty-fifth of February. A US export writes month/day/year, so a sample with only slash dates
/// is read with that form (or day-first when the first component cannot be a month). A file with
/// ISO dates too is left to DuckDB's own inference.
fn slash_date_pattern(path: &Path) -> Option<&'static str> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = vec![0u8; 64 * 1024];
    let read = file.read(&mut buffer).ok()?;
    let sample = String::from_utf8_lossy(&buffer[..read]);

    let mut pattern: Option<&'static str> = None;
    let mut saw_slash = false;
    let mut saw_iso = false;
    for token in sample.split(|character: char| {
        !(character.is_ascii_digit() || character == '/' || character == '-')
    }) {
        if let Some((first, second, third)) = numeric_date_parts(token, '-') {
            if first.len() == 4 && second.len() <= 2 && third.len() <= 2 {
                saw_iso = true;
            }
            continue;
        }
        let Some((first_text, second_text, third_text)) = numeric_date_parts(token, '/') else {
            continue;
        };
        let (Ok(first), Ok(second)) = (first_text.parse::<u32>(), second_text.parse::<u32>())
        else {
            continue;
        };
        saw_slash = true;
        let detected = if first_text.len() == 4 {
            "%Y/%m/%d"
        } else if third_text.len() == 4 {
            if first > 12 && second <= 12 {
                "%d/%m/%Y"
            } else {
                "%m/%d/%Y"
            }
        } else if first > 12 && second <= 12 {
            "%d/%m/%y"
        } else {
            "%m/%d/%y"
        };
        match pattern {
            Some(existing) if existing != detected => return None,
            None => pattern = Some(detected),
            Some(_) => {}
        }
    }
    if saw_iso && saw_slash {
        return None;
    }
    pattern
}

/// Splits a `A<sep>B<sep>C` token into its three numeric parts, or `None` when it is not one.
fn numeric_date_parts(token: &str, separator: char) -> Option<(&str, &str, &str)> {
    let mut parts = token.split(separator);
    let first = parts.next()?;
    let second = parts.next()?;
    let third = parts.next()?;
    if parts.next().is_some() || first.is_empty() || second.is_empty() || third.is_empty() {
        return None;
    }
    if !first.chars().all(|c| c.is_ascii_digit())
        || !second.chars().all(|c| c.is_ascii_digit())
        || !third.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some((first, second, third))
}

/// Runs `DESCRIBE` over a table function and builds the model from it.
fn describe_model(engine: &Engine, source: &str) -> Result<DataModel, ConnectorError> {
    let result = engine.execute(&format!("DESCRIBE SELECT * FROM {source}"), &[])?;

    let mut columns = Vec::new();
    let mut fields = Vec::new();
    let mut used_names = std::collections::HashSet::new();

    for batch in &result.batches {
        let names = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or(ConnectorError::UnexpectedSchema)?;
        let types = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or(ConnectorError::UnexpectedSchema)?;

        for row in 0..batch.num_rows() {
            let name = names.value(row);
            let data_type = map_type(types.value(row));
            let column = Column {
                id: ColumnId::new(format!("col_{}", columns.len())),
                name: name.to_owned(),
                data_type,
                nullable: true,
            };

            let field_id = unique_field_id(name, &mut used_names);
            let field = Field {
                id: field_id,
                name: name.to_owned(),
                table: TableId::new(TABLE_NAME),
                source: FieldSource::Column {
                    column: column.id.clone(),
                },
                role: infer_role(data_type),
                data_type,
                format: None,
                hidden: false,
                description: None,
            };

            columns.push(column);
            fields.push(field);
        }
    }

    Ok(DataModel {
        id: DataModelId::new("m1"),
        name: "Data".to_owned(),
        tables: vec![Table {
            id: TableId::new(TABLE_NAME),
            name: TABLE_NAME.to_owned(),
            columns,
        }],
        fields,
        relationships: vec![],
    })
}

/// Registers a CSV or TSV as a view, cleaning currency-formatted text columns into numbers.
fn open_csv(engine: &Engine, path: &Path) -> Result<DataModel, ConnectorError> {
    let source = source_sql(path, SourceFormat::Csv);
    let mut model = describe_model(engine, &source)?;

    // A textual column is promoted to a number only when every sampled value is a money amount and
    // at least one value carries a currency symbol, a group separator or accounting parentheses.
    let mut cleaned = Vec::new();
    for (index, column) in model.tables[0].columns.iter().enumerate() {
        if !matches!(column.data_type, DataType::Text | DataType::Unknown) {
            continue;
        }
        let quoted = quote_ident(&column.name);
        let sample = sample_text(engine, &source, &quoted)?;
        if let Some(expression) = money_expression(&quoted, &sample) {
            cleaned.push((index, expression));
        }
    }

    let mut selected = Vec::with_capacity(model.tables[0].columns.len());
    for (index, column) in model.tables[0].columns.iter().enumerate() {
        let quoted = quote_ident(&column.name);
        match cleaned.iter().find(|(at, _)| *at == index) {
            Some((_, expression)) => selected.push(format!("{expression} AS {quoted}")),
            None => selected.push(quoted),
        }
    }

    for (index, _) in &cleaned {
        model.tables[0].columns[*index].data_type = DataType::Number;
    }
    let numeric: std::collections::HashSet<ColumnId> = cleaned
        .iter()
        .map(|(index, _)| model.tables[0].columns[*index].id.clone())
        .collect();
    for field in &mut model.fields {
        if let FieldSource::Column { column } = &field.source
            && numeric.contains(column)
        {
            field.data_type = DataType::Number;
            field.role = Role::Measure;
        }
    }

    engine.execute(
        &format!(
            "CREATE OR REPLACE VIEW {} AS SELECT {} FROM {source}",
            quote_ident(TABLE_NAME),
            selected.join(", ")
        ),
        &[],
    )?;
    Ok(model)
}

/// The non-null text values of one column, capped at a sample size that is enough to classify it.
fn sample_text(engine: &Engine, source: &str, column: &str) -> Result<Vec<String>, ConnectorError> {
    let result = engine.execute(
        &format!("SELECT {column} FROM {source} WHERE {column} IS NOT NULL LIMIT 200"),
        &[],
    )?;
    let mut values = Vec::new();
    for batch in &result.batches {
        let Some(array) = batch.column(0).as_any().downcast_ref::<StringArray>() else {
            return Ok(Vec::new());
        };
        for row in 0..array.len() {
            values.push(array.value(row).to_owned());
        }
    }
    Ok(values)
}

/// Returns the SQL expression that reads a money column as a number, when every sample is money.
fn money_expression(column: &str, sample: &[String]) -> Option<String> {
    let mut marked = false;
    let mut count = 0usize;
    for value in sample {
        let text = value.trim();
        if text.is_empty() {
            continue;
        }
        parse_money(text)?;
        if text.contains(['$', '€', '£', '¥', ',', '(', ')']) {
            marked = true;
        }
        count += 1;
    }
    if count == 0 || !marked {
        return None;
    }

    let trimmed = format!("trim({column})");
    // Sign from accounting parentheses or a leading minus; digits and a decimal point from
    // everything else. An empty remainder is NULL, so "n/a" (which would not classify as money)
    // and a bare symbol stay NULL rather than becoming zero.
    Some(format!(
        "TRY_CAST(CASE WHEN {trimmed} LIKE '(%)' OR {trimmed} LIKE '-%' OR {trimmed} LIKE '$-%' \
         THEN -1 ELSE 1 END * TRY_CAST(NULLIF(regexp_replace({trimmed}, '[^0-9.]', '', 'g'), '') \
         AS DOUBLE) AS DOUBLE)"
    ))
}

/// Parses a money amount written in a common English display format.
///
/// Accepts `7.77`, `$7.77`, `-$7.77`, `($7.77)`, `1,234.56` and a leading currency symbol.
pub(crate) fn parse_money(value: &str) -> Option<f64> {
    let mut text = value.trim();
    let mut negative = false;
    if text.starts_with('(') && text.ends_with(')') {
        negative = true;
        text = text[1..text.len() - 1].trim();
    }
    if let Some(rest) = text.strip_prefix('-') {
        negative = !negative;
        text = rest.trim();
    }
    text = text.trim_start_matches(|character: char| {
        matches!(character, '$' | '€' | '£' | '¥') || character.is_whitespace()
    });
    let text = text.replace(',', "");
    let value: f64 = text.parse().ok()?;
    Some(if negative { -value } else { value })
}

/// Whether text looks like money rather than an arbitrary number: it parses and carries a currency
/// symbol, a group separator or accounting parentheses.
pub(crate) fn looks_like_money(value: &str) -> bool {
    parse_money(value).is_some() && value.contains(['$', '€', '£', '¥', ',', '(', ')'])
}

/// Quotes an identifier for SQL, doubling embedded quotes.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Builds the connection that records where this model's data came from.
pub fn connection(path: &Path, format: SourceFormat) -> Connection {
    Connection {
        id: ConnectionId::new("c1"),
        name: file_stem(path),
        kind: match format {
            SourceFormat::Csv => ConnectionKind::Csv,
            SourceFormat::Parquet => ConnectionKind::Parquet,
            SourceFormat::Json => ConnectionKind::Json,
        },
        location: path.to_string_lossy().into_owned(),
        read_only: true,
        options: Default::default(),
    }
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Data".to_owned())
}

fn unique_field_id(name: &str, used: &mut std::collections::HashSet<String>) -> FieldId {
    let base = name;
    if used.insert(base.to_owned()) {
        FieldId::new(base)
    } else {
        let mut index = 2;
        loop {
            let candidate = format!("{base}_{index}");
            if used.insert(candidate.clone()) {
                return FieldId::new(candidate);
            }
            index += 1;
        }
    }
}

fn infer_role(data_type: DataType) -> Role {
    match data_type {
        DataType::Integer | DataType::Number => Role::Measure,
        _ => Role::Dimension,
    }
}

fn map_type(name: &str) -> DataType {
    let name = name.to_ascii_uppercase();
    if is_int(&name) {
        DataType::Integer
    } else if is_float(&name) {
        DataType::Number
    } else if name.starts_with("VARCHAR")
        || name.starts_with("CHAR")
        || name.starts_with("TEXT")
        || name.starts_with("STRING")
        || name.starts_with("ENUM")
        || name.starts_with("UUID")
    {
        DataType::Text
    } else if name.starts_with("BOOLEAN") || name.starts_with("BOOL") {
        DataType::Boolean
    } else if name.starts_with("TIMESTAMP") || name.starts_with("DATETIME") {
        DataType::Timestamp
    } else if name.starts_with("DATE") {
        DataType::Date
    } else {
        DataType::Unknown
    }
}

fn is_int(name: &str) -> bool {
    [
        "TINYINT",
        "SMALLINT",
        "INTEGER",
        "BIGINT",
        "UBIGINT",
        "USMALLINT",
        "UTINYINT",
        "UINTEGER",
    ]
    .iter()
    .any(|kind| name.starts_with(kind))
}

fn is_float(name: &str) -> bool {
    ["HUGEINT", "UHUGEINT", "FLOAT", "DOUBLE", "REAL", "DECIMAL"]
        .iter()
        .any(|kind| name.starts_with(kind))
}

/// Anything that can go wrong while discovering a source.
#[derive(Debug, Error)]
pub enum ConnectorError {
    /// DuckDB could not read or describe the file.
    #[error("cannot read the source: {0}")]
    Engine(#[from] slickbi_engine::EngineError),
    /// The describe result was not the expected shape.
    #[error("the source schema was not in the expected shape")]
    UnexpectedSchema,
    /// A database could not be read.
    #[error("cannot read the database: {0}")]
    Database(String),
    /// A spreadsheet could not be read.
    #[error("cannot read the spreadsheet: {0}")]
    Spreadsheet(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use slickbi_engine::arrow::array::{Date32Array, Float64Array};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp(label: &str) -> std::path::PathBuf {
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "slickbi-connectors-{label}-{count}-{}.csv",
            std::process::id()
        ))
    }

    #[test]
    fn money_formats_are_parsed() {
        assert_eq!(parse_money("7.77"), Some(7.77));
        assert_eq!(parse_money("$7.77"), Some(7.77));
        assert_eq!(parse_money("-$7.77"), Some(-7.77));
        assert_eq!(parse_money("($7.77)"), Some(-7.77));
        assert_eq!(parse_money("$1,234.56"), Some(1234.56));
        assert_eq!(parse_money("n/a"), None);
        assert_eq!(parse_money("Main St"), None);
    }

    #[test]
    fn duckdb_types_map_to_the_application_types() {
        assert_eq!(map_type("BIGINT"), DataType::Integer);
        assert_eq!(map_type("INTEGER"), DataType::Integer);
        assert_eq!(map_type("DOUBLE"), DataType::Number);
        assert_eq!(map_type("DECIMAL(18,2)"), DataType::Number);
        assert_eq!(map_type("HUGEINT"), DataType::Number);
        assert_eq!(map_type("VARCHAR"), DataType::Text);
        assert_eq!(map_type("BOOLEAN"), DataType::Boolean);
        assert_eq!(map_type("DATE"), DataType::Date);
        assert_eq!(map_type("TIMESTAMP WITH TIME ZONE"), DataType::Timestamp);
        assert_eq!(map_type("BLOB"), DataType::Unknown);
    }

    #[test]
    fn numeric_columns_are_measures_and_the_rest_dimensions() {
        assert_eq!(infer_role(DataType::Integer), Role::Measure);
        assert_eq!(infer_role(DataType::Number), Role::Measure);
        assert_eq!(infer_role(DataType::Text), Role::Dimension);
        assert_eq!(infer_role(DataType::Date), Role::Dimension);
    }

    #[test]
    fn a_csv_discovers_typed_fields() {
        let path =
            std::env::temp_dir().join(format!("slickbi-connectors-{}.csv", std::process::id()));
        std::fs::write(&path, "region,amount,units,active,day\nEMEA,10.5,3,true,2025-01-02\nAPAC,4.0,1,false,2025-03-04\n")
            .expect("writes csv");

        let engine = Engine::open().expect("opens");
        let model = discover(&engine, &path, SourceFormat::Csv).expect("discovers");

        assert_eq!(model.tables.len(), 1);
        assert_eq!(model.tables[0].name, "data");
        assert_eq!(model.fields.len(), 5);

        let field = |name: &str| model.fields.iter().find(|f| f.name == name).expect(name);
        assert_eq!(field("region").data_type, DataType::Text);
        assert_eq!(field("region").role, Role::Dimension);
        assert_eq!(field("amount").data_type, DataType::Number);
        assert_eq!(field("amount").role, Role::Measure);
        assert_eq!(field("units").data_type, DataType::Integer);
        assert_eq!(field("active").data_type, DataType::Boolean);
        assert_eq!(field("day").data_type, DataType::Date);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_csv_with_currency_formats_is_cleaned_into_numbers() {
        let path = temp("money");
        std::fs::write(
            &path,
            "day,description,amount,balance,note\n\
             01/02/25,Store,-$7.77,\"$1,234.56\",A-1\n\
             01/03/25,Refund,$5.00,($2.50),B-2\n",
        )
        .expect("writes csv");

        let engine = Engine::open().expect("opens");
        let model = open_file(&engine, &path, SourceFormat::Csv).expect("opens");
        let field = |name: &str| model.fields.iter().find(|f| f.name == name).expect(name);
        assert_eq!(field("day").data_type, DataType::Date);
        assert_eq!(field("description").data_type, DataType::Text);
        assert_eq!(field("amount").data_type, DataType::Number);
        assert_eq!(field("amount").role, Role::Measure);
        assert_eq!(field("balance").data_type, DataType::Number);
        assert_eq!(field("note").data_type, DataType::Text, "not money");

        let result = engine
            .execute(
                "SELECT amount, balance FROM \"data\" ORDER BY day, amount",
                &[],
            )
            .expect("queries the view");
        let batch = &result.batches[0];
        let amounts = batch
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("amount is a number");
        let balances = batch
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("balance is a number");
        assert_eq!(amounts.value(0), -7.77);
        assert_eq!(amounts.value(1), 5.0);
        assert_eq!(balances.value(0), 1234.56);
        assert_eq!(balances.value(1), -2.5);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_tsv_is_read_with_its_tab_delimiter() {
        let path = temp("table");
        std::fs::write(&path, "region\tamount\nEMEA\t$1.50\nAPAC\t$2.25\n").expect("writes tsv");

        let engine = Engine::open().expect("opens");
        let model = open_file(&engine, &path, SourceFormat::Csv).expect("opens");
        let amount = model
            .fields
            .iter()
            .find(|field| field.name == "amount")
            .expect("amount");
        assert_eq!(amount.data_type, DataType::Number);

        let result = engine
            .execute("SELECT sum(amount) FROM \"data\"", &[])
            .expect("queries the view");
        let total = result.batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("sum");
        assert_eq!(total.value(0), 3.75);

        std::fs::remove_file(&path).ok();
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn every_fixture_format_imports_the_same_typed_table() {
        let cases: [(&str, Option<SpreadsheetKind>); 5] = [
            ("transactions.csv", None),
            ("transactions.tsv", None),
            ("transactions.xlsx", Some(SpreadsheetKind::Excel)),
            ("transactions.ods", Some(SpreadsheetKind::OpenDocument)),
            ("transactions.numbers", Some(SpreadsheetKind::Numbers)),
        ];

        for (name, kind) in cases {
            let engine = Engine::open().expect("opens");
            let path = fixture(name);
            let model = match kind {
                Some(kind) => {
                    open_spreadsheet(&engine, &path, kind, None)
                        .unwrap_or_else(|error| panic!("{name}: {error}"))
                        .model
                }
                None => open_file(&engine, &path, SourceFormat::Csv)
                    .unwrap_or_else(|error| panic!("{name}: {error}")),
            };
            let field = |wanted: &str| {
                model
                    .fields
                    .iter()
                    .find(|field| field.name == wanted)
                    .unwrap_or_else(|| panic!("{name} has no {wanted} field"))
            };
            assert_eq!(field("Account").data_type, DataType::Integer, "{name}");
            assert_eq!(field("Description").data_type, DataType::Text, "{name}");
            assert_eq!(field("Day").data_type, DataType::Date, "{name}");
            assert_eq!(field("Day").role, Role::Dimension, "{name}");
            assert_eq!(field("Amount").data_type, DataType::Number, "{name}");
            assert_eq!(field("Amount").role, Role::Measure, "{name}");
            assert_eq!(field("Balance").data_type, DataType::Number, "{name}");
            assert_eq!(field("Derived").data_type, DataType::Number, "{name}");
            assert_eq!(field("Note").data_type, DataType::Text, "{name}");

            let result = engine
                .execute(
                    "SELECT Day, Amount, Balance, Derived FROM \"data\" ORDER BY Day",
                    &[],
                )
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let batch = result.batches.first().unwrap_or_else(|| panic!("{name}"));
            let days = batch
                .column(0)
                .as_any()
                .downcast_ref::<Date32Array>()
                .unwrap_or_else(|| panic!("{name}: a real date column"));
            assert_eq!(
                (days.value(0), days.value(1), days.value(2)),
                (20_090, 20_091, 20_092),
                "{name}: dates"
            );
            let amounts = batch
                .column(1)
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap_or_else(|| panic!("{name}: numeric amounts"));
            assert_eq!(amounts.value(0), -4.25, "{name}");
            assert_eq!(amounts.value(1), 2_000.0, "{name}");
            assert_eq!(amounts.value(2), -1_200.0, "{name}");
            let derived = batch
                .column(3)
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap_or_else(|| panic!("{name}: numeric formula result"));
            assert_eq!(derived.value(0), 4.25, "{name}");
            assert_eq!(derived.value(1), -2_000.0, "{name}");
            assert_eq!(derived.value(2), 1_200.0, "{name}");
        }
    }

    #[test]
    fn the_sqlite_fixture_reads_dates_and_money() {
        use slickbi_engine::{Cell, StorageType};

        let path = fixture("transactions.sqlite");
        let source = DatabaseSource {
            kind: DatabaseKind::Sqlite,
            location: path.to_string_lossy().into_owned(),
            user: None,
            password: None,
            table: "transactions".to_owned(),
            row_limit: 100,
        };
        let table = read_database(&source).expect("reads the SQLite fixture");
        let storage = |wanted: &str| {
            table
                .columns
                .iter()
                .find(|(name, _)| name == wanted)
                .unwrap_or_else(|| panic!("{wanted} column"))
                .1
        };
        assert_eq!(storage("Account"), StorageType::BigInt);
        assert_eq!(storage("Day"), StorageType::Date);
        assert_eq!(storage("Amount"), StorageType::Double);
        assert_eq!(storage("Derived"), StorageType::Double);
        assert_eq!(table.rows[0][0], Cell::Integer(1001));
        assert_eq!(table.rows[0][2], Cell::Date(20_090));
        assert_eq!(table.rows[0][4], Cell::Number(-4.25));
        assert_eq!(table.rows[0][6], Cell::Number(4.25));

        let engine = Engine::open().expect("opens");
        engine
            .register_rows(TABLE_NAME, &table.columns, &table.rows)
            .expect("registers");
        let result = engine
            .execute("SELECT sum(Amount), max(Day) FROM data", &[])
            .expect("queries");
        let batch = &result.batches[0];
        let total = batch
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("sum");
        assert_eq!(total.value(0), 795.75);
        let latest = batch
            .column(1)
            .as_any()
            .downcast_ref::<Date32Array>()
            .expect("max date");
        assert_eq!(latest.value(0), 20_092);
    }

    #[test]
    fn a_spreadsheet_lists_and_selects_its_sheets() {
        let cases = [
            ("transactions.xlsx", SpreadsheetKind::Excel),
            ("transactions.ods", SpreadsheetKind::OpenDocument),
            ("transactions.numbers", SpreadsheetKind::Numbers),
        ];
        for (name, kind) in cases {
            let path = fixture(name);
            let sheets =
                spreadsheet_sheets(&path, kind).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(sheets, vec!["Transactions".to_owned()], "{name}");

            let table = read_spreadsheet(&path, kind, Some("Transactions"))
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(table.sheet, "Transactions", "{name}");
            assert!(
                read_spreadsheet(&path, kind, Some("Missing")).is_err(),
                "{name}: an unknown sheet is refused"
            );
        }
    }
}
