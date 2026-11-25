#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! The query engine: one embedded DuckDB session.
//!
//! The engine is deliberately narrow. It opens an in-memory DuckDB connection, registers source
//! files as tables, executes SQL with parameter values bound (never interpolated), and returns
//! the result as Apache Arrow batches. Arrow is the only interchange format between the engine,
//! the scene and the connectors.
//!
//! It does not know about models, worksheets or charts. A worksheet is planned into SQL by
//! `slickbi-planner`; this crate only runs that SQL. Keeping that seam means a different engine
//! could be substituted without touching the planner.
//!
//! Two promises hold regardless of the workload:
//!
//! * **No hidden network.** Extension auto-load and auto-install are both disabled at open, so a
//!   query can never cause DuckDB to fetch anything.
//! * **Cancellable.** [`Engine::interrupt_handle`] hands out a handle that stops whichever query
//!   is running, from any thread. A cancelled query fails with an error rather than running on.

use std::path::Path;
use std::sync::Arc;

use duckdb::{Connection, InterruptHandle};
use slickbi_calc::ParamValue;
use thiserror::Error;

/// The Apache Arrow crate the engine exchanges batches in.
///
/// Re-exported so the scene and connectors share one Arrow version with the engine and can never
/// disagree about the `RecordBatch` type.
pub use duckdb::arrow;

/// A handle that cancels the query currently running on an engine.
///
/// The handle is `Send` and `Sync`, so it can be moved to the thread that decides to cancel. It
/// stops the query owned by the engine it was obtained from; after that engine is dropped it is a
/// no-op.
pub type Cancellation = Arc<InterruptHandle>;

/// The file formats the engine can read into a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    /// A delimited text file.
    Csv,
    /// An Apache Parquet file.
    Parquet,
    /// A JSON or newline-delimited JSON file.
    Json,
}

/// The storage type of a column imported from a database or another row source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageType {
    /// A 64-bit signed integer.
    BigInt,
    /// A double-precision float.
    Double,
    /// A boolean.
    Boolean,
    /// Variable-length text.
    VarChar,
}

impl StorageType {
    fn sql_name(self) -> &'static str {
        match self {
            StorageType::BigInt => "BIGINT",
            StorageType::Double => "DOUBLE",
            StorageType::Boolean => "BOOLEAN",
            StorageType::VarChar => "VARCHAR",
        }
    }
}

/// One cell in an imported row.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// A SQL NULL.
    Null,
    /// A whole number.
    Integer(i64),
    /// A floating-point number.
    Number(f64),
    /// A boolean.
    Boolean(bool),
    /// Text.
    Text(String),
}

/// A single embedded DuckDB session.
pub struct Engine {
    connection: Connection,
}

impl Engine {
    /// Opens an in-memory engine with extension auto-load and auto-install disabled.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if DuckDB cannot open or cannot apply its safe defaults.
    pub fn open() -> Result<Engine, EngineError> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "SET autoinstall_known_extensions = false;\n\
             SET autoload_known_extensions = false;",
        )?;
        Ok(Engine { connection })
    }

    /// Registers a source file as a table named `name`.
    ///
    /// The table is a view over the file, so every query sees the file's current contents. The
    /// path is bound as a string literal; it is never interpreted as a filesystem path by this
    /// crate beyond being handed to DuckDB.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the file cannot be read as the given format.
    pub fn register(
        &self,
        name: &str,
        path: &Path,
        format: SourceFormat,
    ) -> Result<(), EngineError> {
        let function = match format {
            SourceFormat::Csv => "read_csv_auto",
            SourceFormat::Parquet => "read_parquet",
            SourceFormat::Json => "read_json_auto",
        };
        let sql = format!(
            "CREATE OR REPLACE VIEW {} AS SELECT * FROM {function}('{}')",
            quote_ident(name),
            escape_string(&path.to_string_lossy())
        );
        self.connection
            .execute(&sql, [])
            .map_err(EngineError::from)?;
        Ok(())
    }

    /// Registers rows as a typed table named `name`, replacing any existing table.
    ///
    /// Used for sources that are not files, such as a database read through a driver. The columns
    /// name and type the table; each row must supply one cell per column, in order.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the table cannot be created or a value does not fit its column.
    pub fn register_rows(
        &self,
        name: &str,
        columns: &[(String, StorageType)],
        rows: &[Vec<Cell>],
    ) -> Result<(), EngineError> {
        let definitions = columns
            .iter()
            .map(|(column, storage)| format!("{} {}", quote_ident(column), storage.sql_name()))
            .collect::<Vec<_>>()
            .join(", ");
        self.connection.execute_batch(&format!(
            "CREATE OR REPLACE TABLE {} ({definitions})",
            quote_ident(name)
        ))?;

        let mut appender = self.connection.appender(name)?;
        for row in rows {
            let values: Vec<duckdb::types::Value> = row.iter().map(cell_value).collect();
            appender.append_row(duckdb::appender_params_from_iter(values))?;
        }
        appender.flush()?;
        Ok(())
    }

    /// Executes SQL, binding the parameter values in placeholder order, and returns Arrow batches.
    ///
    /// The SQL is expected to contain one `?` placeholder per value in `params`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the SQL is invalid, if a bound value has the wrong type for its
    /// placeholder, or if the query is cancelled through [`Engine::interrupt_handle`].
    pub fn execute(&self, sql: &str, params: &[ParamValue]) -> Result<QueryResult, EngineError> {
        let mut statement = self.connection.prepare(sql)?;
        let values: Vec<duckdb::types::Value> = params.iter().map(bind).collect();

        let batches: Vec<arrow::record_batch::RecordBatch> = statement
            .stream_arrow(duckdb::params_from_iter(values.iter()))?
            .collect::<Result<Vec<_>, _>>()?;

        let rows = batches.iter().map(|batch| batch.num_rows()).sum();
        Ok(QueryResult { batches, rows })
    }

    /// A handle that cancels the query currently running on this engine.
    pub fn interrupt_handle(&self) -> Cancellation {
        self.connection.interrupt_handle()
    }

    /// Whether extension auto-install is disabled.
    ///
    /// Exposed for tests that assert the no-network guarantee holds, not for the application.
    #[doc(hidden)]
    pub fn autoinstall_disabled(&self) -> Result<bool, EngineError> {
        let value: bool = self
            .connection
            .query_row(
                "SELECT current_setting('autoinstall_known_extensions')",
                [],
                |row| row.get(0),
            )
            .map_err(EngineError::from)?;
        Ok(!value)
    }
}

/// The result of a query, as Arrow batches.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// The batches, in order. May be empty for a query that returns no rows.
    pub batches: Vec<arrow::record_batch::RecordBatch>,
    /// The total number of rows across all batches.
    pub rows: usize,
}

/// Maps a bound value to DuckDB's dynamic value type.
fn bind(value: &ParamValue) -> duckdb::types::Value {
    match value {
        ParamValue::Null => duckdb::types::Value::Null,
        ParamValue::Integer(value) => duckdb::types::Value::BigInt(*value),
        ParamValue::Number(value) => duckdb::types::Value::Double(*value),
        ParamValue::Text(value) => duckdb::types::Value::Text(value.clone()),
        ParamValue::Boolean(value) => duckdb::types::Value::Boolean(*value),
    }
}

/// Maps an imported cell to DuckDB's dynamic value type.
fn cell_value(cell: &Cell) -> duckdb::types::Value {
    match cell {
        Cell::Null => duckdb::types::Value::Null,
        Cell::Integer(value) => duckdb::types::Value::BigInt(*value),
        Cell::Number(value) => duckdb::types::Value::Double(*value),
        Cell::Boolean(value) => duckdb::types::Value::Boolean(*value),
        Cell::Text(value) => duckdb::types::Value::Text(value.clone()),
    }
}

/// Quotes an identifier for SQL, doubling embedded quotes.
fn quote_ident(name: &str) -> String {
    let mut quoted = String::with_capacity(name.len() + 2);
    quoted.push('"');
    for character in name.chars() {
        if character == '"' {
            quoted.push('"');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

/// Escapes a string for use as a single-quoted SQL literal.
fn escape_string(value: &str) -> String {
    value.replace('\'', "''")
}

/// Anything that can go wrong while using the engine.
#[derive(Debug, Error)]
pub enum EngineError {
    /// DuckDB reported an error while opening, registering, or executing.
    #[error("query engine error: {0}")]
    DuckDb(#[from] duckdb::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registering_rows_creates_a_queryable_table() {
        let engine = Engine::open().expect("opens");
        engine
            .register_rows(
                "t",
                &[
                    ("region".to_owned(), StorageType::VarChar),
                    ("amount".to_owned(), StorageType::BigInt),
                ],
                &[
                    vec![Cell::Text("EMEA".into()), Cell::Integer(10)],
                    vec![Cell::Text("APAC".into()), Cell::Integer(5)],
                    vec![Cell::Text("EMEA".into()), Cell::Integer(2)],
                ],
            )
            .expect("registers rows");

        let result = engine
            .execute(
                "SELECT region, sum(amount) AS total FROM t GROUP BY region ORDER BY region",
                &[],
            )
            .expect("queries");
        assert_eq!(result.rows, 2);
        let batch = &result.batches[0];
        let regions = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("text column");
        assert_eq!(regions.value(0), "APAC");
        assert_eq!(regions.value(1), "EMEA");
    }

    #[test]
    fn a_null_cell_is_imported_as_null() {
        let engine = Engine::open().expect("opens");
        engine
            .register_rows(
                "n",
                &[("value".to_owned(), StorageType::BigInt)],
                &[vec![Cell::Null], vec![Cell::Integer(1)]],
            )
            .expect("registers rows");

        let result = engine
            .execute(
                "SELECT count(*) AS total, count(value) AS present FROM n",
                &[],
            )
            .expect("queries");
        let batch = &result.batches[0];
        let total = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .expect("count");
        let present = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .expect("count");
        assert_eq!(total.value(0), 2);
        assert_eq!(present.value(0), 1);
    }
}
