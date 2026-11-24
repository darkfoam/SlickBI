//! Performance fixture: a one-million-row Parquet aggregates quickly.
//!
//! This test is `#[ignore]`d: the strict budget (under 300 ms on a laptop, per
//! `docs/RELEASES.md`) is asserted by the release benchmark, not by wall-clock in the ordinary
//! test run, which would be flaky under parallel CI load. The bound here is deliberately loose —
//! it catches pathological regressions (quadratic plans, accidental re-scans) without failing a
//! busy machine. Run it with `cargo test --release -p slickbi-engine --test perf -- --ignored`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use slickbi_calc::ParamValue;
use slickbi_engine::{Engine, SourceFormat};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_file(label: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("slickbi-perf-{label}-{n}"))
}

#[test]
#[ignore = "performance budget; run with --release -- --ignored"]
fn one_million_rows_aggregate_in_budget() {
    let parquet = temp_file("sales.parquet");

    let engine = Engine::open().expect("opens");
    engine
        .execute(
            "COPY (SELECT (i % 100)::VARCHAR AS region, (i * 0.5)::DOUBLE AS amount \
             FROM range(1000000) t(i)) TO ? (FORMAT PARQUET)",
            &[ParamValue::Text(parquet.to_string_lossy().into_owned())],
        )
        .expect("writes the fixture");

    engine
        .register("sales", &parquet, SourceFormat::Parquet)
        .expect("registers");

    // Warm the plan once so the timed run measures aggregation, not setup.
    let sql = "SELECT region, sum(amount) AS total FROM sales GROUP BY region ORDER BY region";
    engine.execute(sql, &[]).expect("warms up");

    let start = Instant::now();
    let result = engine.execute(sql, &[]).expect("aggregates");
    let elapsed = start.elapsed();

    assert_eq!(result.rows, 100, "one group per region value");
    assert!(
        elapsed.as_millis() < 2_000,
        "one-million-row aggregation took {elapsed:?}, over the loose smoke bound"
    );

    eprintln!("one-million-row aggregation: {elapsed:?}");

    std::fs::remove_file(&parquet).ok();
}
