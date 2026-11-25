//! Database sources: read a table through a native driver and hand it to the engine.
//!
//! The engine is DuckDB, and DuckDB's own database scanners are extensions that this build does
//! not bundle and must not download (see `ARCHITECTURE.md`: no extension auto-load, no hidden
//! network). So a database source is read through a native driver and imported into DuckDB as a
//! typed table: the driver returns every column as text, and the column's declared type decides
//! how the text is parsed.
//!
//! Reading every column as text keeps one code path across SQLite, PostgreSQL and MySQL, and
//! keeps NULL distinct from the empty string. Date and timestamp columns are parsed back into
//! real engine dates and timestamps, so they can drive a continuous time axis.

use std::collections::HashSet;

use slickbi_engine::{Cell, StorageType};
use slickbi_spec::{
    Column, ColumnId, ConnectionKind, DataModel, DataModelId, DataType, Field, FieldSource, Table,
    TableId,
};

use super::{ConnectorError, TABLE_NAME, infer_role, unique_field_id};
use crate::temporal::{parse_date, parse_timestamp};

/// A database kind this build can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseKind {
    /// A SQLite database file.
    Sqlite,
    /// A PostgreSQL server.
    Postgres,
    /// A MySQL or MariaDB server.
    MySql,
}

impl DatabaseKind {
    /// The connection kind recorded in the workbook.
    pub fn connection_kind(self) -> ConnectionKind {
        match self {
            DatabaseKind::Sqlite => ConnectionKind::Sqlite,
            DatabaseKind::Postgres => ConnectionKind::Postgres,
            DatabaseKind::MySql => ConnectionKind::MySql,
        }
    }

    /// A human-readable name for menus and messages.
    pub fn label(self) -> &'static str {
        match self {
            DatabaseKind::Sqlite => "SQLite",
            DatabaseKind::Postgres => "PostgreSQL",
            DatabaseKind::MySql => "MySQL / MariaDB",
        }
    }
}

/// How to reach a database table.
///
/// The password is held in memory only for the duration of the connection; it is never serialised
/// into a workbook or written to a log.
#[derive(Debug, Clone)]
pub struct DatabaseSource {
    /// Which database.
    pub kind: DatabaseKind,
    /// A SQLite file path, or `host[:port]/database` for a server.
    pub location: String,
    /// The user name, for a server.
    pub user: Option<String>,
    /// The password, for a server. Never persisted.
    pub password: Option<String>,
    /// The table to read.
    pub table: String,
    /// The most rows to read.
    pub row_limit: usize,
}

/// A table read from a database, ready to register in the engine and model.
pub struct DatabaseTable {
    /// The semantic model for the table, named [`TABLE_NAME`].
    pub model: DataModel,
    /// The columns and the DuckDB storage type each one uses.
    pub columns: Vec<(String, StorageType)>,
    /// The rows, one cell per column, in column order.
    pub rows: Vec<Vec<Cell>>,
}

/// Reads a database table into a model and typed rows.
///
/// # Errors
///
/// Returns [`ConnectorError::Database`] when the connection fails, the table does not exist, or a
/// value cannot be read.
pub fn read_database(source: &DatabaseSource) -> Result<DatabaseTable, ConnectorError> {
    let raw = match source.kind {
        DatabaseKind::Sqlite => read_sqlite(source)?,
        DatabaseKind::Postgres => read_postgres(source)?,
        DatabaseKind::MySql => read_mysql(source)?,
    };
    Ok(build(source.kind, raw))
}

/// Lists the base tables a database exposes.
///
/// # Errors
///
/// Returns [`ConnectorError::Database`] when the connection fails.
pub fn list_tables(source: &DatabaseSource) -> Result<Vec<String>, ConnectorError> {
    match source.kind {
        DatabaseKind::Sqlite => list_sqlite(source),
        DatabaseKind::Postgres => list_postgres(source),
        DatabaseKind::MySql => list_mysql(source),
    }
}

/// A column as a driver reports it, before typing.
struct RawColumn {
    name: String,
    type_name: String,
}

/// A table as a driver returns it: every value is text or NULL.
struct RawTable {
    columns: Vec<RawColumn>,
    rows: Vec<Vec<Option<String>>>,
}

fn build(kind: DatabaseKind, raw: RawTable) -> DatabaseTable {
    let mut columns = Vec::with_capacity(raw.columns.len());
    let mut storage = Vec::with_capacity(raw.columns.len());
    let mut spec_columns = Vec::with_capacity(raw.columns.len());
    let mut fields = Vec::with_capacity(raw.columns.len());
    let mut used = HashSet::new();

    for (index, column) in raw.columns.iter().enumerate() {
        let (storage_type, data_type) = map_type(kind, &column.type_name);
        let column_id = ColumnId::new(format!("col_{index}"));
        spec_columns.push(Column {
            id: column_id.clone(),
            name: column.name.clone(),
            data_type,
            nullable: true,
        });
        fields.push(Field {
            id: unique_field_id(&column.name, &mut used),
            name: column.name.clone(),
            table: TableId::new(TABLE_NAME),
            source: FieldSource::Column { column: column_id },
            role: infer_role(data_type),
            data_type,
            format: None,
            hidden: false,
            folder: None,
            description: None,
        });
        columns.push((column.name.clone(), storage_type));
        storage.push(storage_type);
    }

    let rows = raw
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .enumerate()
                .map(|(index, value)| parse_cell(storage[index], value))
                .collect()
        })
        .collect();

    DatabaseTable {
        model: DataModel {
            id: DataModelId::new("m1"),
            name: "Data".to_owned(),
            tables: vec![Table {
                id: TableId::new(TABLE_NAME),
                name: TABLE_NAME.to_owned(),
                columns: spec_columns,
            }],
            fields,
            relationships: vec![],
        },
        columns,
        rows,
    }
}

/// Maps a database's declared type name to a DuckDB storage type and an application type.
fn map_type(kind: DatabaseKind, type_name: &str) -> (StorageType, DataType) {
    let name = type_name.to_ascii_lowercase();
    match kind {
        DatabaseKind::Sqlite => {
            if name.contains("int") {
                (StorageType::BigInt, DataType::Integer)
            } else if name.contains("real")
                || name.contains("floa")
                || name.contains("doub")
                || name.contains("dec")
                || name.contains("num")
            {
                (StorageType::Double, DataType::Number)
            } else if name.contains("timestamp") || name.contains("datetime") {
                (StorageType::Timestamp, DataType::Timestamp)
            } else if name.contains("date") {
                (StorageType::Date, DataType::Date)
            } else {
                (StorageType::VarChar, DataType::Text)
            }
        }
        DatabaseKind::Postgres => {
            if matches!(
                name.as_str(),
                "smallint" | "integer" | "bigint" | "serial" | "bigserial" | "smallserial"
            ) {
                (StorageType::BigInt, DataType::Integer)
            } else if matches!(
                name.as_str(),
                "numeric" | "decimal" | "real" | "double precision"
            ) {
                (StorageType::Double, DataType::Number)
            } else if matches!(name.as_str(), "boolean" | "bool") {
                (StorageType::Boolean, DataType::Boolean)
            } else if name == "date" {
                (StorageType::Date, DataType::Date)
            } else if name == "timestamp" || name == "timestamp without time zone" {
                (StorageType::Timestamp, DataType::Timestamp)
            } else {
                (StorageType::VarChar, DataType::Text)
            }
        }
        DatabaseKind::MySql => {
            if name.contains("int") {
                (StorageType::BigInt, DataType::Integer)
            } else if matches!(
                name.as_str(),
                "decimal" | "numeric" | "float" | "double" | "real"
            ) {
                (StorageType::Double, DataType::Number)
            } else if name == "bit" {
                (StorageType::Boolean, DataType::Boolean)
            } else if name == "date" {
                (StorageType::Date, DataType::Date)
            } else if matches!(name.as_str(), "datetime" | "timestamp") {
                (StorageType::Timestamp, DataType::Timestamp)
            } else {
                (StorageType::VarChar, DataType::Text)
            }
        }
    }
}

/// Parses one textual cell into the type its column will hold.
fn parse_cell(storage: StorageType, value: Option<String>) -> Cell {
    let Some(value) = value else {
        return Cell::Null;
    };
    match storage {
        StorageType::BigInt => {
            let text = value.trim();
            if let Ok(number) = text.parse::<i64>() {
                Cell::Integer(number)
            } else if let Ok(number) = text.parse::<f64>() {
                Cell::Integer(number.round() as i64)
            } else {
                Cell::Null
            }
        }
        StorageType::Double => value
            .trim()
            .parse::<f64>()
            .map(Cell::Number)
            .unwrap_or(Cell::Null),
        StorageType::Boolean => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "t" | "1" | "yes" => Cell::Boolean(true),
            "false" | "f" | "0" | "no" => Cell::Boolean(false),
            _ => Cell::Null,
        },
        StorageType::VarChar => Cell::Text(value),
        StorageType::Date => parse_date(&value).map(Cell::Date).unwrap_or(Cell::Null),
        StorageType::Timestamp => parse_timestamp(&value)
            .map(Cell::Timestamp)
            .unwrap_or(Cell::Null),
    }
}

fn row_limit(limit: usize) -> usize {
    limit.clamp(1, 5_000_000)
}

/// Splits `host[:port]/database` into its parts, applying the driver's default port.
fn split_location(location: &str, default_port: u16) -> (String, u16, String) {
    let (authority, database) = match location.rsplit_once('/') {
        Some((authority, database)) => (authority, database.to_owned()),
        None => (location, String::new()),
    };
    match authority.rsplit_once(':') {
        Some((host, port)) if port.parse::<u16>().is_ok() => (
            host.to_owned(),
            port.parse().unwrap_or(default_port),
            database,
        ),
        _ => (authority.to_owned(), default_port, database),
    }
}

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

fn quote_ident_mysql(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn database_error(error: impl std::fmt::Display) -> ConnectorError {
    ConnectorError::Database(error.to_string())
}

// ---- SQLite ----

fn sqlite_read(source: &DatabaseSource) -> Result<rusqlite::Connection, ConnectorError> {
    rusqlite::Connection::open(&source.location).map_err(database_error)
}

fn list_sqlite(source: &DatabaseSource) -> Result<Vec<String>, ConnectorError> {
    let connection = sqlite_read(source)?;
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(database_error)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(database_error)?;
    let mut tables = Vec::new();
    for row in rows {
        tables.push(row.map_err(database_error)?);
    }
    Ok(tables)
}

fn read_sqlite(source: &DatabaseSource) -> Result<RawTable, ConnectorError> {
    let connection = sqlite_read(source)?;

    let mut info = connection
        .prepare(&format!(
            "PRAGMA table_info({})",
            quote_ident(&source.table)
        ))
        .map_err(database_error)?;
    let columns: Vec<RawColumn> = info
        .query_map([], |row| {
            Ok(RawColumn {
                name: row.get(1)?,
                type_name: row.get(2)?,
            })
        })
        .map_err(database_error)?
        .collect::<Result<_, _>>()
        .map_err(database_error)?;
    if columns.is_empty() {
        return Err(ConnectorError::Database(format!(
            "SQLite has no table named '{}'",
            source.table
        )));
    }

    let select = columns
        .iter()
        .map(|column| format!("CAST({} AS TEXT)", quote_ident(&column.name)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {select} FROM {} LIMIT {}",
        quote_ident(&source.table),
        row_limit(source.row_limit)
    );

    let mut statement = connection.prepare(&sql).map_err(database_error)?;
    let count = columns.len();
    let rows: Vec<Vec<Option<String>>> = statement
        .query_map([], |row| {
            (0..count)
                .map(|index| row.get::<_, Option<String>>(index))
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(database_error)?
        .collect::<Result<_, _>>()
        .map_err(database_error)?;

    Ok(RawTable { columns, rows })
}

// ---- PostgreSQL ----

fn postgres_connect(source: &DatabaseSource) -> Result<(postgres::Client, String), ConnectorError> {
    let (host, port, database) = split_location(&source.location, 5432);
    let mut config = postgres::Config::new();
    config.host(&host).port(port).dbname(&database);
    if let Some(user) = &source.user {
        config.user(user);
    }
    if let Some(password) = &source.password {
        config.password(password);
    }
    let client = config.connect(postgres::NoTls).map_err(database_error)?;
    Ok((client, database))
}

fn list_postgres(source: &DatabaseSource) -> Result<Vec<String>, ConnectorError> {
    let (mut client, _) = postgres_connect(source)?;
    let rows = client
        .query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY table_name",
            &[],
        )
        .map_err(database_error)?;
    Ok(rows.iter().map(|row| row.get::<_, String>(0)).collect())
}

fn read_postgres(source: &DatabaseSource) -> Result<RawTable, ConnectorError> {
    let (mut client, _) = postgres_connect(source)?;

    let columns: Vec<RawColumn> = client
        .query(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = $1 ORDER BY ordinal_position",
            &[&source.table],
        )
        .map_err(database_error)?
        .iter()
        .map(|row| RawColumn {
            name: row.get(0),
            type_name: row.get(1),
        })
        .collect();
    if columns.is_empty() {
        return Err(ConnectorError::Database(format!(
            "PostgreSQL has no table named '{}' in schema 'public'",
            source.table
        )));
    }

    let select = columns
        .iter()
        .map(|column| format!("{}::text", quote_ident(&column.name)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {select} FROM {} LIMIT {}",
        quote_ident(&source.table),
        row_limit(source.row_limit)
    );

    let rows = client.query(&sql, &[]).map_err(database_error)?;
    let count = columns.len();
    let rows = rows
        .iter()
        .map(|row| {
            (0..count)
                .map(|index| row.get::<_, Option<String>>(index))
                .collect()
        })
        .collect();

    Ok(RawTable { columns, rows })
}

// ---- MySQL / MariaDB ----

fn mysql_connect(source: &DatabaseSource) -> Result<mysql::Conn, ConnectorError> {
    let (host, port, database) = split_location(&source.location, 3306);
    let mut builder = mysql::OptsBuilder::new()
        .ip_or_hostname(Some(host))
        .tcp_port(port);
    if !database.is_empty() {
        builder = builder.db_name(Some(database));
    }
    if let Some(user) = &source.user {
        builder = builder.user(Some(user));
    }
    if let Some(password) = &source.password {
        builder = builder.pass(Some(password));
    }
    mysql::Conn::new(builder).map_err(database_error)
}

fn list_mysql(source: &DatabaseSource) -> Result<Vec<String>, ConnectorError> {
    use mysql::prelude::Queryable;
    let mut connection = mysql_connect(source)?;
    let tables: Vec<String> = connection
        .query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' ORDER BY table_name",
        )
        .map_err(database_error)?;
    Ok(tables)
}

fn read_mysql(source: &DatabaseSource) -> Result<RawTable, ConnectorError> {
    use mysql::prelude::Queryable;
    let mut connection = mysql_connect(source)?;

    let columns: Vec<RawColumn> = connection
        .exec::<(String, String), _, _>(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = ? ORDER BY ordinal_position",
            (source.table.as_str(),),
        )
        .map_err(database_error)?
        .into_iter()
        .map(|(name, type_name)| RawColumn { name, type_name })
        .collect();
    if columns.is_empty() {
        return Err(ConnectorError::Database(format!(
            "MySQL has no table named '{}' in this database",
            source.table
        )));
    }

    let select = columns
        .iter()
        .map(|column| format!("CAST({} AS CHAR)", quote_ident_mysql(&column.name)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {select} FROM {} LIMIT {}",
        quote_ident_mysql(&source.table),
        row_limit(source.row_limit)
    );

    let rows: Vec<mysql::Row> = connection.query(&sql).map_err(database_error)?;
    let count = columns.len();
    let rows = rows
        .iter()
        .map(|row| {
            (0..count)
                .map(|index| mysql_cell(row, index))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(RawTable { columns, rows })
}

fn mysql_cell(row: &mysql::Row, index: usize) -> Result<Option<String>, ConnectorError> {
    match row.get_opt::<Option<String>, usize>(index) {
        Some(Ok(value)) => Ok(value),
        Some(Err(error)) => Err(database_error(error)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slickbi_spec::Role;

    fn temp_db(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "slickbi-db-{label}-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn sqlite_source(path: &std::path::Path) -> DatabaseSource {
        DatabaseSource {
            kind: DatabaseKind::Sqlite,
            location: path.to_string_lossy().into_owned(),
            user: None,
            password: None,
            table: "sales".to_owned(),
            row_limit: 1_000,
        }
    }

    #[test]
    fn a_sqlite_table_is_read_and_typed() {
        let path = temp_db("read");
        {
            let connection = rusqlite::Connection::open(&path).expect("opens");
            connection
                .execute_batch(
                    "CREATE TABLE sales (region TEXT, amount INTEGER, ratio REAL);
                     INSERT INTO sales VALUES ('EMEA', 10, 1.5), ('APAC', 5, 2.0), ('EMEA', 2, NULL);",
                )
                .expect("seeds");
        }

        let table = read_database(&sqlite_source(&path)).expect("reads");
        assert_eq!(table.columns.len(), 3);
        assert_eq!(
            table.columns[0],
            ("region".to_owned(), StorageType::VarChar)
        );
        assert_eq!(table.columns[1], ("amount".to_owned(), StorageType::BigInt));
        assert_eq!(table.columns[2], ("ratio".to_owned(), StorageType::Double));

        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[0][0], Cell::Text("EMEA".into()));
        assert_eq!(table.rows[0][1], Cell::Integer(10));
        assert_eq!(table.rows[0][2], Cell::Number(1.5));
        assert_eq!(table.rows[2][2], Cell::Null);

        let amount = table
            .model
            .fields
            .iter()
            .find(|field| field.name == "amount")
            .expect("amount field");
        assert_eq!(amount.data_type, DataType::Integer);
        assert_eq!(amount.role, Role::Measure);

        let region = table
            .model
            .fields
            .iter()
            .find(|field| field.name == "region")
            .expect("region field");
        assert_eq!(region.data_type, DataType::Text);
        assert_eq!(region.role, Role::Dimension);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_sqlite_date_column_is_read_as_a_date() {
        let path = temp_db("date");
        {
            let connection = rusqlite::Connection::open(&path).expect("opens");
            connection
                .execute_batch(
                    "CREATE TABLE sales (day DATE, amount INTEGER);
                     INSERT INTO sales VALUES ('2025-01-01', 10), ('2025-02-01', 5);",
                )
                .expect("seeds");
        }

        let table = read_database(&sqlite_source(&path)).expect("reads");
        assert_eq!(table.columns[0], ("day".to_owned(), StorageType::Date));
        assert_eq!(table.rows[0][0], Cell::Date(20_089));
        assert_eq!(table.model.fields[0].data_type, DataType::Date);
        assert_eq!(table.model.fields[0].role, Role::Dimension);

        let engine = slickbi_engine::Engine::open().expect("opens");
        engine
            .register_rows(TABLE_NAME, &table.columns, &table.rows)
            .expect("registers");
        let result = engine
            .execute("SELECT day FROM data ORDER BY day", &[])
            .expect("queries");
        let days = result.batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<slickbi_engine::arrow::array::Date32Array>()
            .expect("a date column");
        assert_eq!(days.value(0), 20_089);
        assert_eq!(days.value(1), 20_120);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn sqlite_lists_its_base_tables() {
        let path = temp_db("list");
        {
            let connection = rusqlite::Connection::open(&path).expect("opens");
            connection
                .execute_batch("CREATE TABLE zeta (a INTEGER); CREATE TABLE alpha (b TEXT);")
                .expect("seeds");
        }

        let source = sqlite_source(&path);
        let tables = list_tables(&source).expect("lists");
        assert_eq!(tables, vec!["alpha".to_owned(), "zeta".to_owned()]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn the_engine_can_query_a_read_sqlite_table() {
        let path = temp_db("engine");
        {
            let connection = rusqlite::Connection::open(&path).expect("opens");
            connection
                .execute_batch(
                    "CREATE TABLE sales (region TEXT, amount INTEGER);
                     INSERT INTO sales VALUES ('EMEA', 10), ('APAC', 5), ('EMEA', 2);",
                )
                .expect("seeds");
        }

        let table = read_database(&sqlite_source(&path)).expect("reads");
        let engine = slickbi_engine::Engine::open().expect("opens");
        engine
            .register_rows(TABLE_NAME, &table.columns, &table.rows)
            .expect("registers");

        let result = engine
            .execute(
                "SELECT region, sum(amount) AS total FROM data GROUP BY region ORDER BY region",
                &[],
            )
            .expect("queries");
        assert_eq!(result.rows, 2);
        std::fs::remove_file(&path).ok();
    }
}
