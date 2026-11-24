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
