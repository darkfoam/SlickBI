//! Database connector integration tests.
//!
//! Each test skips when its environment variable is absent, so `cargo test` stays green on a
//! machine with no database server; point the variable at a real server to run it:
//!
//! ```sh
//! SLICKBI_TEST_PG=127.0.0.1:55432/demo SLICKBI_TEST_PG_USER=slickbi SLICKBI_TEST_PG_PASS=secret \
//!   cargo test -p slickbi-connectors --test databases
//! ```
//!
//! The expected table is created by the test harness and has the shape:
//! `sales(region text, amount int, ratio float, ok boolean)` with two rows.

use slickbi_connectors::{DatabaseKind, DatabaseSource, list_tables, read_database};
use slickbi_engine::Cell;

fn source(kind: DatabaseKind, location: String, user_var: &str, pass_var: &str) -> DatabaseSource {
    DatabaseSource {
        kind,
        location,
        user: std::env::var(user_var).ok(),
        password: std::env::var(pass_var).ok(),
        table: "sales".to_owned(),
        row_limit: 1_000,
    }
}

#[test]
fn postgres_reads_a_table() {
    let Ok(location) = std::env::var("SLICKBI_TEST_PG") else {
        return;
    };
    let source = source(
        DatabaseKind::Postgres,
        location,
        "SLICKBI_TEST_PG_USER",
        "SLICKBI_TEST_PG_PASS",
    );

    let tables = list_tables(&source).expect("lists PostgreSQL tables");
    assert!(tables.iter().any(|table| table == "sales"), "{tables:?}");

    let table = read_database(&source).expect("reads PostgreSQL");
    assert_eq!(table.columns.len(), 4);
    assert_eq!(table.columns[0].0, "region");
    assert_eq!(table.columns[1].0, "amount");
    assert_eq!(table.rows.len(), 2);
    assert!(
        table
            .rows
            .iter()
            .any(|row| row[0] == Cell::Text("EMEA".to_owned())),
        "rows: {:?}",
        table.rows
    );
}

#[test]
fn mysql_reads_a_table() {
    let Ok(location) = std::env::var("SLICKBI_TEST_MYSQL") else {
        return;
    };
    let source = source(
        DatabaseKind::MySql,
        location,
        "SLICKBI_TEST_MYSQL_USER",
        "SLICKBI_TEST_MYSQL_PASS",
    );

    let tables = list_tables(&source).expect("lists MySQL tables");
    assert!(tables.iter().any(|table| table == "sales"), "{tables:?}");

    let table = read_database(&source).expect("reads MySQL");
    assert_eq!(table.columns.len(), 4);
    assert_eq!(table.columns[0].0, "region");
    assert_eq!(table.rows.len(), 2);
    assert!(
        table
            .rows
            .iter()
            .any(|row| row[0] == Cell::Text("EMEA".to_owned())),
        "rows: {:?}",
        table.rows
    );
}

#[test]
fn postgres_answers_a_live_query() {
    let Ok(location) = std::env::var("SLICKBI_TEST_PG") else {
        return;
    };
    let source = source(
        DatabaseKind::Postgres,
        location,
        "SLICKBI_TEST_PG_USER",
        "SLICKBI_TEST_PG_PASS",
    );
    let columns = vec![
        ("x".to_owned(), slickbi_spec::DataType::Text),
        ("y".to_owned(), slickbi_spec::DataType::Integer),
    ];
    let sql = "SELECT \"region\" AS \"x\", sum(\"amount\") AS \"y\" FROM \"sales\" \
               GROUP BY 1 ORDER BY 1";
    let result = slickbi_connectors::query_live(&source, sql, &[], &columns)
        .expect("answers the live query");
    assert_eq!(result.rows, 2);
    let batch = &result.batches[0];
    use slickbi_engine::arrow::array::{Array, StringArray};
    let regions = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("text");
    assert_eq!(regions.len(), 2);
    assert!(regions.iter().flatten().any(|region| region == "EMEA"));
}

#[test]
fn mysql_answers_a_live_query() {
    let Ok(location) = std::env::var("SLICKBI_TEST_MYSQL") else {
        return;
    };
    let source = source(
        DatabaseKind::MySql,
        location,
        "SLICKBI_TEST_MYSQL_USER",
        "SLICKBI_TEST_MYSQL_PASS",
    );
    let columns = vec![
        ("x".to_owned(), slickbi_spec::DataType::Text),
        ("y".to_owned(), slickbi_spec::DataType::Integer),
    ];
    let sql = "SELECT \"region\" AS \"x\", sum(\"amount\") AS \"y\" FROM \"sales\" \
               GROUP BY 1 ORDER BY 1";
    let result = slickbi_connectors::query_live(&source, sql, &[], &columns)
        .expect("answers the live query");
    assert_eq!(result.rows, 2);
}
