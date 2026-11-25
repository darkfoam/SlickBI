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

pub use database::{DatabaseKind, DatabaseSource, DatabaseTable, list_tables, read_database};

/// The name a discovered table registers under.
pub const TABLE_NAME: &str = "data";

/// Discovers the schema of a file and builds a single-table model over it.
///
/// # Errors
///
/// Returns [`ConnectorError`] if the file cannot be read or described.
pub fn discover(
    engine: &Engine,
    path: &Path,
    format: SourceFormat,
) -> Result<DataModel, ConnectorError> {
    let function = match format {
        SourceFormat::Csv => "read_csv_auto",
        SourceFormat::Parquet => "read_parquet",
        SourceFormat::Json => "read_json_auto",
    };
    let sql = format!(
        "DESCRIBE SELECT * FROM {function}('{}')",
        path.to_string_lossy().replace('\'', "''")
    );
    let result = engine.execute(&sql, &[])?;

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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
