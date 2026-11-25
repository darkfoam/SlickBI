//! Live queries against a database source.
//!
//! The planner emits DuckDB-shaped SQL (`"identifiers"`, `?` placeholders, `GROUP BY` positions).
//! SQLite and PostgreSQL accept that shape as written; MySQL does after `ANSI_QUOTES` makes the
//! double quote an identifier quote. The statement runs on the source through its native driver,
//! every result column is cast to text in a wrapping query, and the text is parsed back into the
//! Arrow type the planner declared. No value is ever interpolated into SQL.
//!
//! A construct the source cannot execute is refused by type before the query is sent; the caller
//! never receives numbers computed a different way.

use std::sync::Arc;

use slickbi_calc::ParamValue;
use slickbi_engine::QueryResult;
use slickbi_engine::arrow::array::{
    ArrayRef, BooleanArray, Date32Array, Float64Array, Int64Array, StringArray,
    TimestampMicrosecondArray,
};
use slickbi_engine::arrow::datatypes::{DataType as ArrowDataType, Field as ArrowField, Schema};
use slickbi_engine::arrow::record_batch::RecordBatch;
use slickbi_spec::DataType;

use crate::database::{
    DatabaseKind, DatabaseSource, mysql_connect, postgres_connect, quote_ident, quote_ident_mysql,
    sqlite_read,
};
use crate::temporal::{parse_date, parse_timestamp};

/// Runs one planned statement against the source and returns its rows as Arrow batches.
///
/// `columns` is the planner's output list, in `SELECT` order; each name is the alias the statement
/// uses and each type is the Arrow type the scene expects.
///
/// # Errors
///
/// Returns [`LiveError`] when the source rejects the statement, a value does not fit its declared
/// type, or the Arrow batch cannot be built.
pub fn query(
    source: &DatabaseSource,
    sql: &str,
    params: &[ParamValue],
    columns: &[(String, DataType)],
) -> Result<QueryResult, LiveError> {
    if columns.is_empty() {
        return Err(LiveError::EmptyColumns);
    }
    let wrapped = wrap(source.kind, sql, columns);
    let wrapped = match source.kind {
        DatabaseKind::Postgres => postgres_placeholders(&wrapped),
        _ => wrapped,
    };
    let rows = match source.kind {
        DatabaseKind::Sqlite => query_sqlite(source, &wrapped, params, columns.len())?,
        DatabaseKind::Postgres => query_postgres(source, &wrapped, params, columns.len())?,
        DatabaseKind::MySql => query_mysql(source, &wrapped, params, columns.len())?,
    };
    let batch = batch(&rows, columns)?;
    let count = batch.num_rows();
    Ok(QueryResult {
        batches: vec![batch],
        rows: count,
    })
}

/// Wraps the planned statement so every output column comes back as text.
fn wrap(kind: DatabaseKind, sql: &str, columns: &[(String, DataType)]) -> String {
    let cast = match kind {
        DatabaseKind::MySql => "CHAR",
        _ => "TEXT",
    };
    let projection = columns
        .iter()
        .map(|(name, _)| {
            let quoted = match kind {
                DatabaseKind::MySql => quote_ident_mysql(name),
                _ => quote_ident(name),
            };
            format!("CAST({quoted} AS {cast}) AS {quoted}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("SELECT {projection} FROM ({sql}) AS slickbi_live")
}

fn query_sqlite(
    source: &DatabaseSource,
    sql: &str,
    params: &[ParamValue],
    count: usize,
) -> Result<Vec<Vec<Option<String>>>, LiveError> {
    let connection = sqlite_read(source).map_err(source_error)?;
    let mut statement = connection.prepare(sql).map_err(source_error)?;
    let values: Vec<rusqlite::types::Value> = params.iter().map(sqlite_value).collect();
    let rows = statement
        .query_map(rusqlite::params_from_iter(values), |row| {
            (0..count)
                .map(|index| row.get::<_, Option<String>>(index))
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(source_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(source_error)
}

fn sqlite_value(value: &ParamValue) -> rusqlite::types::Value {
    match value {
        ParamValue::Null => rusqlite::types::Value::Null,
        ParamValue::Integer(value) => rusqlite::types::Value::Integer(*value),
        ParamValue::Number(value) => rusqlite::types::Value::Real(*value),
        ParamValue::Text(value) => rusqlite::types::Value::Text(value.clone()),
        ParamValue::Boolean(value) => rusqlite::types::Value::Integer(i64::from(*value)),
    }
}

fn query_postgres(
    source: &DatabaseSource,
    sql: &str,
    params: &[ParamValue],
    count: usize,
) -> Result<Vec<Vec<Option<String>>>, LiveError> {
    use postgres::types::ToSql;
    let (mut client, _) = postgres_connect(source).map_err(source_error)?;
    let values: Vec<Box<dyn ToSql + Sync>> = params.iter().map(postgres_value).collect();
    let refs: Vec<&(dyn ToSql + Sync)> = values.iter().map(|value| value.as_ref()).collect();
    let rows = client.query(sql, &refs).map_err(source_error)?;
    Ok(rows
        .iter()
        .map(|row| {
            (0..count)
                .map(|index| row.get::<_, Option<String>>(index))
                .collect()
        })
        .collect())
}

fn postgres_value(value: &ParamValue) -> Box<dyn postgres::types::ToSql + Sync> {
    match value {
        ParamValue::Null => Box::new(Option::<String>::None),
        ParamValue::Integer(value) => Box::new(*value),
        ParamValue::Number(value) => Box::new(*value),
        ParamValue::Text(value) => Box::new(value.clone()),
        ParamValue::Boolean(value) => Box::new(*value),
    }
}

fn query_mysql(
    source: &DatabaseSource,
    sql: &str,
    params: &[ParamValue],
    count: usize,
) -> Result<Vec<Vec<Option<String>>>, LiveError> {
    use mysql::prelude::Queryable;
    let mut connection = mysql_connect(source).map_err(source_error)?;
    // The planner quotes identifiers with `"`. MySQL treats that as a string unless ANSI_QUOTES
    // is on, which is the mode that makes the planned SQL portable.
    connection
        .query_drop("SET SESSION sql_mode = CONCAT_WS(',', @@sql_mode, 'ANSI_QUOTES')")
        .map_err(source_error)?;
    let values: Vec<mysql::Value> = params.iter().map(mysql_value).collect();
    let mut result = connection
        .exec_iter(sql, mysql::Params::Positional(values))
        .map_err(source_error)?;
    let mut rows = Vec::new();
    for row in result.by_ref() {
        let row = row.map_err(source_error)?;
        let mut cells = Vec::with_capacity(count);
        for index in 0..count {
            cells.push(match row.get_opt::<Option<String>, usize>(index) {
                Some(Ok(value)) => value,
                Some(Err(error)) => return Err(LiveError::Source(error.to_string())),
                None => None,
            });
        }
        rows.push(cells);
    }
    Ok(rows)
}

fn mysql_value(value: &ParamValue) -> mysql::Value {
    match value {
        ParamValue::Null => mysql::Value::NULL,
        ParamValue::Integer(value) => mysql::Value::Int(*value),
        ParamValue::Number(value) => mysql::Value::Double(*value),
        ParamValue::Text(value) => mysql::Value::Bytes(value.clone().into_bytes()),
        ParamValue::Boolean(value) => mysql::Value::Int(i64::from(*value)),
    }
}

fn source_error(error: impl std::fmt::Display) -> LiveError {
    LiveError::Source(error.to_string())
}

/// Rewrites the planner's `?` placeholders as PostgreSQL's `$1`, `$2`, …, skipping text literals.
///
/// Live mode refuses calculated fields, which are the only place a text literal can reach the
/// statement; the scan is still quote-aware so a `?` in a string can never become a placeholder.
fn postgres_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 8);
    let mut index = 0usize;
    let mut in_text = false;
    for character in sql.chars() {
        match character {
            '\'' => {
                in_text = !in_text;
                out.push(character);
            }
            '?' if !in_text => {
                index += 1;
                out.push('$');
                out.push_str(&index.to_string());
            }
            other => out.push(other),
        }
    }
    out
}

/// Builds one Arrow batch from text rows, using the planner's declared column types.
fn batch(
    rows: &[Vec<Option<String>>],
    columns: &[(String, DataType)],
) -> Result<RecordBatch, LiveError> {
    let mut fields = Vec::with_capacity(columns.len());
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(columns.len());

    for (index, (name, data_type)) in columns.iter().enumerate() {
        let cell = |row: &Vec<Option<String>>| row.get(index).cloned().flatten();
        match data_type {
            DataType::Integer => {
                let values = rows
                    .iter()
                    .map(|row| match cell(row) {
                        Some(value) => parse_integer(&value, name).map(Some),
                        None => Ok(None),
                    })
                    .collect::<Result<Vec<Option<i64>>, _>>()?;
                arrays.push(Arc::new(Int64Array::from(values)));
                fields.push(ArrowField::new(name, ArrowDataType::Int64, true));
            }
            DataType::Number => {
                let values = rows
                    .iter()
                    .map(|row| match cell(row) {
                        Some(value) => parse_number(&value, name).map(Some),
                        None => Ok(None),
                    })
                    .collect::<Result<Vec<Option<f64>>, _>>()?;
                arrays.push(Arc::new(Float64Array::from(values)));
                fields.push(ArrowField::new(name, ArrowDataType::Float64, true));
            }
            DataType::Boolean => {
                let values = rows
                    .iter()
                    .map(|row| match cell(row) {
                        Some(value) => parse_boolean(&value, name).map(Some),
                        None => Ok(None),
                    })
                    .collect::<Result<Vec<Option<bool>>, _>>()?;
                arrays.push(Arc::new(BooleanArray::from(values)));
                fields.push(ArrowField::new(name, ArrowDataType::Boolean, true));
            }
            DataType::Date => {
                let values = rows
                    .iter()
                    .map(|row| match cell(row) {
                        Some(value) => {
                            parse_date(&value)
                                .map(Some)
                                .ok_or_else(|| LiveError::Value {
                                    column: name.clone(),
                                    kind: "date",
                                    value,
                                })
                        }
                        None => Ok(None),
                    })
                    .collect::<Result<Vec<Option<i32>>, _>>()?;
                arrays.push(Arc::new(Date32Array::from(values)));
                fields.push(ArrowField::new(name, ArrowDataType::Date32, true));
            }
            DataType::Timestamp => {
                let values = rows
                    .iter()
                    .map(|row| match cell(row) {
                        Some(value) => {
                            parse_timestamp(&value)
                                .map(Some)
                                .ok_or_else(|| LiveError::Value {
                                    column: name.clone(),
                                    kind: "timestamp",
                                    value,
                                })
                        }
                        None => Ok(None),
                    })
                    .collect::<Result<Vec<Option<i64>>, _>>()?;
                arrays.push(Arc::new(TimestampMicrosecondArray::from(values)));
                fields.push(ArrowField::new(
                    name,
                    ArrowDataType::Timestamp(
                        slickbi_engine::arrow::datatypes::TimeUnit::Microsecond,
                        None,
                    ),
                    true,
                ));
            }
            DataType::Text | DataType::Unknown => {
                let values: Vec<Option<String>> = rows.iter().map(cell).collect();
                arrays.push(Arc::new(StringArray::from(values)));
                fields.push(ArrowField::new(name, ArrowDataType::Utf8, true));
            }
        }
    }

    Ok(RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)?)
}

fn parse_integer(value: &str, column: &str) -> Result<i64, LiveError> {
    let text = value.trim();
    text.parse::<i64>()
        .or_else(|_| text.parse::<f64>().map(|number| number.round() as i64))
        .map_err(|_| LiveError::Value {
            column: column.to_owned(),
            kind: "integer",
            value: value.to_owned(),
        })
}

fn parse_number(value: &str, column: &str) -> Result<f64, LiveError> {
    value.trim().parse::<f64>().map_err(|_| LiveError::Value {
        column: column.to_owned(),
        kind: "number",
        value: value.to_owned(),
    })
}

fn parse_boolean(value: &str, column: &str) -> Result<bool, LiveError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "t" | "true" | "yes" => Ok(true),
        "0" | "f" | "false" | "no" => Ok(false),
        _ => Err(LiveError::Value {
            column: column.to_owned(),
            kind: "boolean",
            value: value.to_owned(),
        }),
    }
}

/// Anything that can go wrong while running a live query.
#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    /// A calculated field is on a shelf, and the source cannot run its expression.
    #[error("live mode cannot run calculated field '{0}'; use an extract or a snapshot")]
    CalculatedField(String),
    /// A field's type was overridden, and its cast is not portable to the source.
    #[error("live mode cannot run the type override on field '{0}'; use an extract")]
    TypeOverride(String),
    /// The source does not implement the aggregation.
    #[error("{kind} cannot run {construct} in live mode; use an extract")]
    Unsupported {
        /// The source kind, for the message.
        kind: &'static str,
        /// The construct, for the message.
        construct: &'static str,
    },
    /// The source rejected the statement or the connection failed.
    #[error("the live source failed: {0}")]
    Source(String),
    /// A result value does not fit the type the planner declared.
    #[error("the live result for column '{column}' is not a {kind}: '{value}'")]
    Value {
        /// The column alias.
        column: String,
        /// The expected type.
        kind: &'static str,
        /// The value the source returned.
        value: String,
    },
    /// The planner produced no output columns.
    #[error("the live query has no output columns")]
    EmptyColumns,
    /// The Arrow batch could not be built.
    #[error("the live result could not be assembled: {0}")]
    Arrow(#[from] slickbi_engine::arrow::error::ArrowError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use slickbi_engine::arrow::array::Array;

    fn temp(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "slickbi-live-{label}-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn a_live_sqlite_query_returns_typed_rows() {
        let path = temp("query");
        {
            let connection = rusqlite::Connection::open(&path).expect("opens");
            connection
                .execute_batch(
                    "CREATE TABLE sales (region TEXT, amount INTEGER, day DATE);
                     INSERT INTO sales VALUES
                       ('EMEA', 10, '2025-01-02'),
                       ('APAC', 5, '2025-01-03'),
                       ('EMEA', 2, '2025-01-04');",
                )
                .expect("seeds");
        }
        let source = DatabaseSource {
            kind: DatabaseKind::Sqlite,
            location: path.to_string_lossy().into_owned(),
            user: None,
            password: None,
            table: "sales".into(),
            row_limit: 100,
        };
        let columns = vec![
            ("x".to_owned(), DataType::Text),
            ("y".to_owned(), DataType::Integer),
            ("z".to_owned(), DataType::Date),
        ];
        let sql = "SELECT \"region\" AS \"x\", sum(\"amount\") AS \"y\", max(\"day\") AS \"z\" \
                   FROM \"sales\" WHERE \"region\" = ? GROUP BY 1 ORDER BY 1";

        let result = query(&source, sql, &[ParamValue::Text("EMEA".into())], &columns)
            .expect("queries the source");
        assert_eq!(result.rows, 1);
        let batch = &result.batches[0];
        let region = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("text");
        assert_eq!(region.value(0), "EMEA");
        let total = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("integer");
        assert_eq!(total.value(0), 12);
        let latest = batch
            .column(2)
            .as_any()
            .downcast_ref::<Date32Array>()
            .expect("date");
        assert_eq!(latest.value(0), 20_092);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn postgres_placeholders_are_numbered_and_quotes_are_safe() {
        assert_eq!(
            postgres_placeholders("SELECT a FROM t WHERE b = ? AND c = ?"),
            "SELECT a FROM t WHERE b = $1 AND c = $2"
        );
        assert_eq!(
            postgres_placeholders("SELECT '?' AS q WHERE b = ?"),
            "SELECT '?' AS q WHERE b = $1"
        );
    }

    #[test]
    fn the_wrapper_casts_every_output_to_text() {
        let columns = vec![
            ("x".to_owned(), DataType::Text),
            ("y".to_owned(), DataType::Number),
        ];
        let sql = "SELECT \"region\" AS \"x\", sum(\"amount\") AS \"y\" FROM \"data\"";
        let wrapped = wrap(DatabaseKind::Sqlite, sql, &columns);
        assert!(wrapped.starts_with("SELECT CAST(\"x\" AS TEXT) AS \"x\""));
        assert!(wrapped.contains("CAST(\"y\" AS TEXT) AS \"y\""));
        assert!(wrapped.contains("FROM (SELECT"));

        let mysql = wrap(DatabaseKind::MySql, sql, &columns);
        assert!(mysql.contains("CAST(`x` AS CHAR) AS `x`"));
    }

    #[test]
    fn text_rows_become_typed_arrow_columns() {
        let columns = vec![
            ("label".to_owned(), DataType::Text),
            ("amount".to_owned(), DataType::Number),
            ("count".to_owned(), DataType::Integer),
            ("active".to_owned(), DataType::Boolean),
            ("day".to_owned(), DataType::Date),
        ];
        let rows = vec![
            vec![
                Some("EMEA".into()),
                Some("13.75".into()),
                Some("3".into()),
                Some("t".into()),
                Some("2025-01-02".into()),
            ],
            vec![None, None, None, None, None],
        ];
        let batch = batch(&rows, &columns).expect("batch");
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 5);
        let label = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("text");
        assert_eq!(label.value(0), "EMEA");
        assert!(label.is_null(1));
        let amount = batch
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("number");
        assert_eq!(amount.value(0), 13.75);
        let count = batch
            .column(2)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("integer");
        assert_eq!(count.value(0), 3);
        let active = batch
            .column(3)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .expect("boolean");
        assert!(active.value(0));
        let day = batch
            .column(4)
            .as_any()
            .downcast_ref::<Date32Array>()
            .expect("date");
        assert_eq!(day.value(0), 20_090);
    }

    #[test]
    fn a_bad_value_is_not_silently_null() {
        let columns = vec![("amount".to_owned(), DataType::Number)];
        let rows = vec![vec![Some("not a number".into())]];
        let error = batch(&rows, &columns).expect_err("must refuse");
        assert!(matches!(error, LiveError::Value { .. }));
    }
}
