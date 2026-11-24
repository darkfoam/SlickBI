# ADR 0001 — Query engine: DuckDB

- **Status:** accepted
- **Date:** 2026-11-24
- **Deciders:** nclack

## Context

SlickBI is a local-first desktop BI tool. The query engine must, on the user's own machine:

- run an analytical SQL workload (aggregations, filters, sorts) over files the user already owns;
- read CSV, Parquet and JSON without importing them into a proprietary store;
- support cancellation, because the UI supersedes a query as fast as a shelf changes;
- never touch the network by itself;
- impose no artificial cap on data size (spill to disk, stream, no in-memory limit);
- build and run identically on Windows, macOS and Linux;
- be embeddable in a closed-source, shrink-wrapped desktop binary.

## Decision

Use **DuckDB**, embedded in-process via the `duckdb` crate (bundled amalgamation, DuckDB 2.0).
`parquet` and `json` readers are statically linked. Arrow is the only result format. Extension
auto-load and auto-install are disabled at open.

SlickBI never writes a `.duckdb` file: it queries the user's files in place, so there is no
engine-owned store, no format lock-in, and no storage cap.

## Alternatives considered

| Engine | Verdict |
|---|---|
| DataFusion | Credible successor, but an engine kit: SlickBI would own the catalog and persistence. Breaking releases every ~11 weeks. Rejected for a small team. |
| Polars | SQL is a convenience layer over the expression API; no catalog; two query paths. Rejected. |
| chDB | No Windows support, ~120 MB library, experimental Rust bindings, downloads a binary at build time. Rejected. |
| SQLite | Row-store, single-threaded, 10–100× slower on analytics, no native Parquet, no `QUALIFY`/`PIVOT`. Kept only for app state (recents, layout, cache) and as a source connector — never the query engine or the workbook format. |
| GlareDB | Abandoned (Nov 2025). Rejected. |
| Velox | No SQL frontend. Rejected. |
| MonetDBLite | Discontinued. Rejected. |

## Consequences

- **Positive.** One engine covers all three OSes; full SQL surface (`PIVOT`, `QUALIFY`, window
  functions) is available later without an engine swap; cancellation is a first-class
  `InterruptHandle`; spills to disk so there is no size cap.
- **Negative.** The bundled C++ build is slow on first build and complicates cross-compilation
  (the wasm build in 0.0.6 needs `duckdb-wasm`). A DuckDB connection is not `Sync`, so the engine
  is single-threaded with a query queue.
- **Boundaries.** The engine interface is deliberately narrow (open, register, execute, interrupt)
  so another engine can be substituted without touching the planner.
