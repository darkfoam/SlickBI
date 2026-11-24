//! Integration tests for the engine, against a real in-memory DuckDB.

use std::path::PathBuf;

use slickbi_calc::ParamValue;
use slickbi_engine::arrow::array::{Array, Float64Array, Int64Array, StringArray};
use slickbi_engine::{Engine, EngineError, SourceFormat};

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("slickbi-engine-{name}-{}.tmp", std::process::id()))
}

fn write_csv(path: &PathBuf) {
    std::fs::write(
        path,
        "region,amount,units\nEMEA,100.5,10\nAPAC,40.0,4\nEMEA,25.0,3\n",
    )
    .expect("write csv");
}

#[test]
fn an_engine_reads_a_csv_and_aggregates() {
    let path = temp_path("sales.csv");
    write_csv(&path);

    let engine = Engine::open().expect("opens");
    engine
        .register("sales", &path, SourceFormat::Csv)
        .expect("registers");

    let result = engine
        .execute(
            "SELECT region, sum(amount) AS total FROM sales GROUP BY region ORDER BY region",
            &[],
        )
        .expect("queries");

    assert_eq!(result.rows, 2);
    let batch = &result.batches[0];

    let regions = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("region is text");
    let totals = batch
        .column(1)
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("total is double");

    assert_eq!(regions.value(0), "APAC");
    assert_eq!(regions.value(1), "EMEA");
    assert_eq!(totals.value(0), 40.0);
    assert_eq!(totals.value(1), 125.5);
}

#[test]
fn parameters_are_bound_not_interpolated() {
    let path = temp_path("params.csv");
    write_csv(&path);

    let engine = Engine::open().expect("opens");
    engine
        .register("sales", &path, SourceFormat::Csv)
        .expect("registers");

    let result = engine
        .execute(
            "SELECT region FROM sales WHERE amount > ? ORDER BY region",
            &[ParamValue::Number(50.0)],
        )
        .expect("queries");

    assert_eq!(result.rows, 1);
    let batch = &result.batches[0];
    let regions = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("region is text");
    assert_eq!(regions.value(0), "EMEA");
}

#[test]
fn a_text_value_cannot_break_out_of_its_placeholder() {
    let path = temp_path("inject.csv");
    write_csv(&path);

    let engine = Engine::open().expect("opens");
    engine
        .register("sales", &path, SourceFormat::Csv)
        .expect("registers");

    // A malicious value is still just a bound string, never part of the SQL.
    let result = engine.execute(
        "SELECT region FROM sales WHERE region = ?",
        &[ParamValue::Text("EMEA'; DROP TABLE sales; --".into())],
    );

    assert!(result.is_ok());
    assert_eq!(result.expect("result").rows, 0);
}

#[test]
fn the_engine_reads_json() {
    let path = temp_path("rows.json");
    std::fs::write(
        &path,
        "{\"region\":\"EMEA\",\"amount\":100.5}\n{\"region\":\"APAC\",\"amount\":40.0}\n",
    )
    .expect("write json");

    let engine = Engine::open().expect("opens");
    engine
        .register("sales", &path, SourceFormat::Json)
        .expect("registers");

    let result = engine
        .execute("SELECT count(*) AS n FROM sales", &[])
        .expect("queries");
    let batch = &result.batches[0];
    let count = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("count is bigint");
    assert_eq!(count.value(0), 2);
}

#[test]
fn the_engine_reads_parquet() {
    let path = temp_path("extract.parquet");

    let engine = Engine::open().expect("opens");
    // DuckDB writes the Parquet, so this also proves the parquet reader is linked.
    engine
        .execute(
            "COPY (SELECT 'EMEA' AS region, 100.5 AS amount) TO ? (FORMAT PARQUET)",
            &[ParamValue::Text(path.to_string_lossy().into_owned())],
        )
        .expect("writes parquet");

    engine
        .register("extract", &path, SourceFormat::Parquet)
        .expect("registers");

    let result = engine
        .execute("SELECT region, amount FROM extract", &[])
        .expect("queries");
    assert_eq!(result.rows, 1);
    let batch = &result.batches[0];
    let region = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("region");
    assert_eq!(region.value(0), "EMEA");
}

#[test]
fn extension_autoload_and_autoinstall_are_off() {
    let engine = Engine::open().expect("opens");
    assert!(
        engine.autoinstall_disabled().expect("reads setting"),
        "autoinstall_known_extensions must be disabled at open"
    );
}

#[test]
fn an_invalid_query_is_reported() {
    let engine = Engine::open().expect("opens");
    let error = engine
        .execute("SELECT nonsense FROM missing", &[])
        .expect_err("must fail");
    assert!(matches!(error, EngineError::DuckDb(_)));
}

#[test]
fn a_query_can_be_cancelled() {
    let engine = Engine::open().expect("opens");
    let handle = engine.interrupt_handle();

    let worker = std::thread::spawn(move || {
        engine.execute(
            "SELECT count(*) FROM range(0, 100000000) a, range(0, 100000000) b",
            &[],
        )
    });

    std::thread::sleep(std::time::Duration::from_millis(200));
    handle.interrupt();

    let result = worker.join().expect("thread joins");
    assert!(result.is_err(), "an interrupted query must fail");
}
