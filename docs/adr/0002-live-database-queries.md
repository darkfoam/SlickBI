# ADR 0002 — Live queries against database sources

- **Status:** proposed
- **Date:** 2026-11-25
- **Deciders:** nclack

## Context

D15 asks for a **live query mode against the source**. Today a database source is read once through
its native driver (`rusqlite`, `postgres`, `mysql`), typed, and registered in DuckDB as a table.
Worksheet queries then run in DuckDB. File sources (CSV, Parquet, JSON) are different: they are
registered as views, so they are already live.

DuckDB cannot scan SQLite, PostgreSQL or MySQL without an extension, and this product forbids
extension downloads and hidden network access (ADR 0001). The planner, model and calculation
compiler emit DuckDB SQL with bound `?` parameters; `Dialect` exists but has only `DuckDb`.

A live path must not produce different numbers from the imported path. A construct the source
dialect cannot express must be refused **by type**, not retried or silently computed another way.

## Decision (proposed)

Add a `Dialect` (`Sqlite`, `Postgres`, `MySql`) through three layers, then execute the same logical
plan on the native driver and convert its rows to Arrow:

1. **Model.** Render identifiers with the dialect's quote (`"` or backtick), render override casts
   with the dialect's type names, and cast a date/timestamp parameter where the dialect needs it
   (PostgreSQL will not compare a `date` to a `text` placeholder).
2. **Planner.** Quote aliases for the dialect and know which aggregates it has. `median`,
   `stddev_samp` and `var_samp` are absent from SQLite and `median` from MySQL.
3. **Calculations.** Lower `substr`, `concat`, `date_trunc` and the other functions to the dialect,
   or return a typed refusal when there is no equivalent. A live source does not silently fall
   back to a snapshot.
4. **Executor.** `connectors::live::query(source, sql, params) -> Result<QueryResult, LiveError>`
   runs the statement through the driver. Every result column is cast to text in the statement
   wrapper and parsed to the planner's `OutputColumn` type: `Int64`, `Float64`, `String`,
   `Boolean`, `Date32`, `TimestampMicrosecond`.
5. **Mode.** `Connection.options["mode"]` is `live` or `snapshot`. Live is opt-in until the parity
   suite passes for that dialect, then the default for new connections. The toolbar shows which
   mode a source is in, and a refusal names the field, construct and dialect and offers "Use an
   extract".
6. **Persistence.** Saving still writes a Parquet extract, so a workbook opens offline. SQLite can
   reconnect by its file path when the workbook opens; a server source asks for credentials, which
   are never stored.

## Delivery order

| Phase | Work | Exit |
|---|---|---|
| 1 | SQLite dialect, executor, mode switch | Parity suite over every shelf, aggregation, filter and sort; a row inserted into the database changes the chart after `refresh` |
| 2 | PostgreSQL dialect and parameter casts | Same suite against a real server, including date filters |
| 3 | MySQL dialect and quoting | Same suite against MariaDB |
| 4 | Push filters, aggregation and limit to the source | The live plan reads only the rows a chart can show; a 1M-row table charts within the budget |
| 5 | Live becomes the default; snapshot stays an explicit choice | Opening a workbook with credentials reconnects; without them it opens the extract |

## Parity suite

For each dialect, a small fixture table is read both ways (live and imported) and every query is
compared on the planned columns, the exact values, the Arrow types and the scene's marks. Negative
cases: an unsupported aggregate, a calculated field the dialect cannot lower, a cancelled query, a
dropped connection and a legitimate source error that must not be swallowed. Both paths are tested
for a construct that already works, not just the one that failed.

## Alternatives considered

| Option | Verdict |
|---|---|
| Re-read the whole source on every render | No dialect work, but it is a full scan per keystroke and is not query pushdown. Rejected as live mode; it is the snapshot refresh (D17). |
| DuckDB `sqlite`/`postgres`/`mysql` extensions | Forbidden: they download at first use and break the no-hidden-network promise. Rejected. |
| Snapshot only, de-scope D15 | Honest but leaves a defined MVP feature unfinished; record it as a scope change instead of calling it done. |
| Client-side computation over raw rows | Moves kernel decisions (split, aggregate, type) into the application, against the layer rule. Rejected. |

## Consequences

- **Positive.** Real freshness and pushdown for database sources, with one planner and one scene.
- **Negative.** The dialect touches model, calc and planner, and needs three driver adapters. Date,
  decimal and boolean text forms differ per driver; the parity suite is the guard.
- **Risk.** Live and snapshot can diverge on a construct added later. The typed refusal and the
  parity suite are the mitigation; adding an aggregate means adding its availability per dialect.
