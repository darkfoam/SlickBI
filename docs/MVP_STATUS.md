# MVP status map — 0.0.3 "Workbench"

Snapshot: 2026-11-25. Updated after the database sources, spreadsheets and the continuous time
axis landed.

This is a **derived** map of what stands between the current tree and a completed MVP. The
authoritative documents remain [`FEATURES.md`](FEATURES.md) (per-feature status),
[`MVP.md`](MVP.md) (acceptance criteria), [`RELEASES.md`](RELEASES.md) (scope and exit criteria)
and [`TESTING.md`](TESTING.md) (test obligations). Where this map disagrees with `FEATURES.md`,
the disagreement is called out in §5 — it means a status needs updating, not that the map is
right.

Status legend: **Done** = implemented, wired into the app/CLI and covered by tests. **Partial** =
some layer exists (often the planner or the model) but a required layer or the UI is missing.
**Not started** = no meaningful implementation.

Effort to close: **S** ≈ hours, **M** ≈ a few days, **L** ≈ one to two weeks, **XL** = external
infrastructure or a new dependency.

---

## 1. What "completed MVP" requires

A feature is not enough on its own. The MVP ships when **all four** are true:

1. **Feature scope** — everything in the 0.0.1, 0.0.2 and 0.0.3 scopes is present
   (`RELEASES.md`). §2 covers this.
2. **Acceptance criteria** — MVP-01 … MVP-20 pass, asserted by tests (`MVP.md` §5). §3 covers this.
3. **Quality bars** — the §6 budgets are measured and enforced. §4 covers this.
4. **Test obligations** — the §8 test requirements and the `TESTING.md` boundary map are met, and
   the §10 ship checklist is checked on all three platforms.

At the snapshot, of the **51 MVP-scope features**: **51 done, 0 partial, 0 not started**.

---

## 2. Feature map (scope = 0.0.1 + 0.0.2 + 0.0.3)

### Data sources

| ID | Feature | Status | Reality / gap | Effort |
|---|---|---|---|---|
| D01 | CSV and TSV + drag and drop | **Done** | `connectors::open_file` registers a live view; currency strings and `MM/DD/YY` dates are typed correctly | — |
| D02 | Excel workbooks + sheet selection | **Done** | `calamine` reads `.xlsx`/`.xlsm`/`.xlsb`/`.xls`; a sheet picker appears when a workbook has more than one | — |
| D03 | Parquet | **Done** | `read_parquet` discovery | — |
| D04 | JSON / NDJSON | **Done** | `read_json_auto` | — |
| D05 | SQLite | **Done** | Native `rusqlite` driver; typed rows imported into DuckDB (dates now real dates); tested | — |
| D06 | PostgreSQL | **Done** | Native `postgres` driver; verified against a real server | — |
| D07 | MySQL / MariaDB | **Done** | Native `mysql` driver; verified against MariaDB | — |
| D14 | Local extract to Parquet | **Done** | `Session::export_extract` + an Export menu item write the source table as Parquet | — |
| D15 | Live query mode | **Done** | Worksheet queries run on SQLite/PostgreSQL/MySQL while connected; calculations, overrides and a few operators are refused by type; snapshot is a connection choice | — |
| D18 | OpenDocument spreadsheets + sheet selection | **Done** | `calamine` reads `.ods` through the same typed importer as Excel | — |
| D19 | Apple Numbers documents + table selection | **Done** | `iwork` reads the first table, or the one the user picks | — |

### Semantic model

| ID | Feature | Status | Reality / gap | Effort |
|---|---|---|---|---|
| M01 | Auto typing into dims/measures | **Done** | `connectors::infer_role`; data pane split | — |
| M02 | Rename / hide / reorder / folders | **Done** | Rename rewrites calculation references; hide/unhide; move within a group; folders group the data pane | — |
| M03 | Type override, number and date formats | **Done** | Type override casts in SQL; number and date formats; dates are categories in bars, continuous on line/area/point | — |
| M04 | Field descriptions | **Done** | Editable from the data-pane menu, shown as a hover tooltip | — |

### Calculations

| ID | Feature | Status | Reality / gap | Effort |
|---|---|---|---|---|
| C01 | Arithmetic, comparison, IF/CASE, text, date | **Done** | `slickbi-calc` lexer/parser/typing/lowering | — |
| C02 | Aggregations incl. distinct/median | **Done** | `Agg` → SQL; engine tests | — |
| C03 | Parameters in calculations and filters | **Done** | `Workbook.parameters`, `[Name]` in calculations, filter binding, editor | — |
| C04 | Type checking with inline, spanning errors | **Done** | The calculation editor highlights the offending span | — |

### Charts and worksheets

| ID | Feature | Status | Reality / gap | Effort |
|---|---|---|---|---|
| V01 | Bar and column charts | **Done** | Vertical and horizontal, grouped by colour (`5268723`) | — |
| V02 | Line and area charts | **Done** | Multi-series by colour | — |
| V03 | Scatter plots | **Done** | Multi-series by colour | — |
| V04 | Tables | **Done** | `Mark::Table`: headers, formatted cells, a header separator, in all renderers | — |
| V05 | Shelves: row, column, colour, size, text, detail, tooltip | **Done** | All seven; size scales points, text labels marks, detail splits, tooltip feeds hover | — |
| V06 | Axis, legend and tooltip configuration | **Done** | Axis titles, legend/axes visibility, hover tooltips driven by the tooltip shelf | — |
| V07 | Sort and row limit | **Done** | toolbar + planner `ORDER BY`/`LIMIT` | — |
| V08 | Heatmaps | **Done** | `Mark::Rect`: two categorical axes, a sequential blue ramp on the colour shelf, cell tooltips, both renderers | — |
| V09 | Pie and donut | **Done** | Pie (`a6d91c9`) and donut (`f45f9de`), both with legends | — |
| V10 | KPI and text tiles | **Done** | `Mark::Text` renders one formatted value, no axes needed | — |
| V11 | Colourblind-safe palettes by default | **Done** | Okabe–Ito default palette; a test keeps the scene and theme in sync | — |
| V12 | Automatic chart suggestion | **Done** | A toolbar hint suggests the mark the shelves imply; applying it is one click | — |
| V23 | Continuous time axis for dates | **Done** | Temporal columns become numeric ticks with date-formatted ticks on line/area/point; bars and pies keep date categories. The motivating case is a bank balance over a month | — |
| T01 | Tabs: create/rename/duplicate/close/reorder | **Done** | reorder added in `007cb37` | — |
| T03 | "Used in" indicator | **Done** | `Session::used_in`, toolbar display | — |
| T04 | Fork a worksheet | **Done** | `duplicate_worksheet` | — |
| T05 | Delete safety + placeholder | **Done** | warn + placeholder; validator no longer rejects dangling refs (`e6193dd`) | — |
| T06 | Jump from report element to worksheet | **Done** | Clicking a report chart opens its source worksheet | — |

### Reports

| ID | Feature | Status | Reality / gap | Effort |
|---|---|---|---|---|
| R01 | Report canvas / grid | **Done** | canvas, auto-grid placement | — |
| R02 | Live worksheet references | **Done** | scene rebuilt from the worksheet each frame | — |
| R03 | Move, resize, reorder elements | **Done** | `caa7a17`, clamped + tested | — |
| R04 | Text / markdown elements | **Done** | text blocks | — |
| R05 | Per-element presentation overrides | **Done** | title/legend/axes/padding/border (`d8fa9b3`) | — |
| R06 | Cross-filtering between elements | **Done** | Clicking a mark filters the report's other charts; the title opens the worksheet; empty space or Clear restores | — |

### Data out, automation, format, platform

| ID | Feature | Status | Reality / gap | Effort |
|---|---|---|---|---|
| X01 | Raw CSV/Parquet export | **Done** | app + CLI; but see MVP-17 (not byte-identical to DuckDB) | — |
| A01 | CLI validate/compile/query/inspect | **Done** | — | — |
| A02 | CLI render worksheet **or report** | **Done** | `slickbi-cli render-report` composes chart and text elements via `render_report_png` | — |
| O01 | Published `.scz` specification | **Done** | `docs/FORMAT.md` | — |
| O02 | Generated JSON Schema, published | **Done** | `docs/workbook.schema.json`, `slickbi-cli schema`, drift + output-validation tests (MVP-20) | — |
| O04 | Versioning and migration policy | **Done** | Policy in `docs/FORMAT.md`; `from_json_str` migrates through a named seam | — |
| U07 | Dark and light themes | **Done** | Light/dark switch in the top bar | — |
| U08 | Onboarding sample workbook | **Done** | Six months of sample sales, two charts (bar and time line) and a report, loaded on first run | — |

---

## 3. Acceptance criteria (MVP-01 … MVP-20)

| ID | Criterion (short) | Verdict | Gap to close |
|---|---|---|---|
| 01 | 500k CSV → fields < 3 s, no modal | **Partial** | Works; startup/ingest budget not measured |
| 02 | Postgres field list + chart < 2 s | **Partial** | Connects through a native driver; first-chart budget not measured |
| 03 | Rows/cols chart == hand-written GROUP BY | **Met** | golden SQL + engine tests |
| 04 | Sum→Avg == AVG | **Met** | `changing_the_aggregation_changes_the_values` |
| 05 | Filter totals == WHERE | **Met** | filter tests + golden SQL |
| 06 | Parameter bound to filter re-renders | **Met** | C03: parameter list, `[Name]` in calculations, filter binding |
| 07 | Divide-by-zero calc: null semantics, editor warns | **Partial** | SQL null behaviour holds; editor does not surface the risk |
| 08 | Type error inline at span, not saved | **Met** | Inline span highlight; invalid expressions are not added |
| 09 | Rename updates report panel + used-in | **Met** | name read live |
| 10 | Delete used worksheet → placeholders, warns, names reports | **Met** | T05 fix + test |
| 11 | Worksheet filter change updates open report | **Met** | report rebuilds from worksheet each frame |
| 12 | Element title override affects only that report | **Met** | R05 + rendering |
| 13 | Click bar → other charts cross-filter; empty clears | **Met** | R06: mark hit test, transient report selection, predicate on the other elements |
| 14 | Report reflows 1400→900 with no overlap/clip | **Partial** | Canvas scales; no overflow clip now, but reflow/determinism not tested at 900 px |
| 15 | Save/quit/reopen identical | **Met** | round-trip tests |
| 16 | 0.0.2 workbook opens via migration | **Partial** | No migration path/fixture (single format version) |
| 17 | CSV export byte-identical to DuckDB writer | **Not met** | Hand-rolled CSV in app and CLI (quoting/format can differ) |
| 18 | `slickbi-cli render` report == on-screen within tolerance | **Partial** | `render-report` composes the canvas to PNG; no image-diff harness yet |
| 19 | 8 h soak, < 10 % memory growth | **Not met** | No soak test |
| 20 | JSON Schema validates app output, zero errors | **Met** | `jsonschema` validates the serialised workbook; schema file cannot drift |

Counts: **Met 12 · Partial 4 · Not met 4.**

---

## 4. Quality bars (§6) and test obligations (§8 / TESTING.md)

| Obligation | State |
|---|---|
| Cold start < 2 s | Not measured |
| Chart render < 300 ms at 1M rows | `engine/tests/perf.rs` exists but is `#[ignore]` and asserts < 2 s, not the 300 ms budget; not in the CI gate |
| Report re-render < 500 ms | Not measured |
| Filter/cross-filter < 500 ms p95 | Not measured (cross-filter absent) |
| No reachable panics; typed errors | Largely met in core crates; not audited |
| Memory < 1 GB at 1M rows | Not measured |
| Binary < 120 MB | Not measured |
| Golden SQL per shape | **Present** — `crates/planner/tests/sql/duckdb/*.sql` (13 shapes) |
| Golden image / render parity (GPU vs export) | **Missing** — no image-diff harness; app uses egui, CLI uses `tiny-skia` |
| Fixture corpus ≥ 20 workbooks (incl. malformed) | **Missing** — one round-trip test, no corpus |
| UI test through an egui harness | **Missing** — no `egui_kittest`/harness |
| Soak test (1M rows, 8 h) | **Missing** |
| Dependency audit / licence / wasm / binary-size CI gates | Only fmt, clippy, boundaries, tests run (`xtask ci`) |

---

## 5. Where `FEATURES.md` lags the code (fix the statuses)

No known drift at this snapshot. The items previously listed here — V06, V11, A02, O01/O02/O04,
D14 and U07 — were closed in the tree and their statuses updated.

---

## 6. Path to completion (dependency-ordered)

Each phase is independently shippable and leaves `cargo xtask ci` green.

### Phase 1 — quick wins (finish half-built surface)
- [x] **V09 donut** — inner radius on the pie scene; mark choice in the toolbar. **S**
- [x] **M04 descriptions** — editor + tooltip; needs no format change. **S**
- [x] **T06 jump to source** — click a report element to activate its worksheet. **S**
- [x] **M02 rename, hide, reorder and folders** — reference rewriting, data-pane folders. **M**
- [x] **V06 axis titles + tooltips** — done. **M**
- [x] **V11 palette verification** — Okabe–Ito default; scene/theme kept in sync by a test. **S**
- [x] **U07 dark mode** — light/dark switch in the top bar (persistence not yet). **S**
- [x] **U08 onboarding workbook** — sample sales, two charts and a report; loaded on first run. **S**

### Phase 2 — chart types (each needs the scene + both renderers)
- [x] **V10 KPI/text tiles** — scene for `Mark::Text` (planner already plans it). **M**
- [x] **V08 heatmaps** — `Mark::Rect`, sequential colour ramp, two categorical axes. **M**
- [x] **V04 tables** — `Mark::Table`, text-grid scene, all renderers, UI. **L**
- [x] **V05 remaining shelves** — size/text/detail/tooltip through the scene and renderers. **M**
- [x] **V12 chart suggestion** — the shelves suggest a mark; one click applies it. **M**
- [x] **V23 continuous time axis** — temporal columns map to numeric ticks with date-formatted
      ticks on line, area and point charts; bars and pies keep date categories. **S**

### Phase 3 — analytical core
- [x] **C03 parameters** — `Workbook.parameters`, `[Name]` in calculations, filter binding, UI. **L**
- [x] **C04 inline span errors** — highlight the offending range in the calc editor. **M**
- [x] **M03 remainder** — type override; date formatting and date dimensions. **M**

### Phase 4 — reports and testing seams
- [x] **R06 cross-filtering** — mark hit test, transient selection, predicate per element. **L**
- [x] **A02 report render in the CLI** — `slickbi-cli render-report` composes a report to PNG. **M**
- [ ] **Image-diff harness** between the on-screen renderer and the export renderer + a
      deterministic reflow test at 1400/900 px. Unblocks **MVP-14** and **MVP-18**. **M**

### Phase 5 — sources (external dependencies / infrastructure)
- [x] **D05 SQLite** — native `rusqlite` driver: read as text, typed into DuckDB. **L**
- [x] **D06 Postgres** — native `postgres` driver, verified against a real server. **XL**
- [x] **D07 MySQL / MariaDB** — native `mysql` driver, verified against MariaDB. **XL**
- [x] **D02 Excel** — `calamine` reads workbooks; sheet picker. **L**
- [x] **D18 / D19 ODS and Numbers** — `calamine` and `iwork` feed the same typed importer. **M**
- [x] **D14 extract to Parquet** — `Session::save` embeds one; an explicit Export action exists. **M**
- [x] **D15 live DB mode** — planned SQL on the native driver, Arrow conversion, typed refusals, live/snapshot choice. **L**
- [ ] Credentials in the OS keychain — currently session-only, and never persisted. **M**

### Phase 6 — format, acceptance and release hardening
- [x] **O01** spec document, **O02** publish the schema and validate the app's own output
      (**MVP-20**), **O04** migration policy. **M**
- [ ] **MVP-17** — export CSV through DuckDB's writer so it is byte-identical. **S**
- [ ] **MVP-19** — soak test + memory budget; promote the perf fixture to a real 300 ms gate. **M**
- [ ] **MVP-14** — a deterministic reflow test at 1400/900 px.
- [ ] **MVP-01 / MVP-02** — measure ingest and first-chart budgets once the DB sources exist.
- [ ] Fixture corpus (≥ 20 workbooks) and UI harness obligations from `TESTING.md`. **L**

### Phase 7 — ship
- [ ] Run the §10 checklist on macOS, Windows and Linux.
- [ ] The ten-minute demo script, by a non-author, on a clean machine.
- [ ] Update `FEATURES.md` to `done` for every shipped ID; write the changelog.
- [ ] Cut `release/0.0.3`, `cargo xtask ci` green, merge to `main`, tag, merge back to `dev`.

---

## 7. Definition of done (from `MVP.md` §10)

- [ ] Every acceptance criterion passes on all three platforms
- [ ] Demo script under ten minutes by a non-author
- [ ] Format spec and JSON Schema published
- [ ] Changelog written
- [ ] `FEATURES.md` statuses updated
- [ ] `cargo xtask ci` green on the release branch
- [ ] No ignored tests, warnings, or TODOs without IDs
- [ ] Tag `0.0.3` on `main`, merged back to `dev`

The largest risks to the MVP are the remaining V12/R06/U08 scope and the acceptance-hardening work. The
cheapest large wins are Phase 1 (finish visible, half-built surface) and Phase 6 (acceptance
hardening: CSV parity, schema validation, report render parity, soak gate).
