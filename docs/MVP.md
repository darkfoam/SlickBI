# MVP — 0.3 "Workbench"

The first version that is a product rather than a demonstration. Someone who did not write it
can install nothing, open a workbook, and be productive.

**Single sentence:** a person can load their own data, build individual charts as tabs, compose
those charts into a styled report, and have the report stay current when the charts change.

---

## 1. Who it is for

The small-business analyst from [`PRODUCT.md`](PRODUCT.md), on their own machine, with a
spreadsheet or a Postgres database, who needs an answer and a document they can send to someone.

They are not a developer. They will not read documentation before their first chart. They will
close the app if the first five minutes are confusing.

## 2. What ships

| Area | Feature IDs |
|---|---|
| Sources | D01 CSV, D02 Excel, D03 Parquet, D04 JSON, D05 SQLite, D06 Postgres, D07 MySQL, D14 extracts, D15 live mode |
| Model | M01 auto typing, M02 rename/hide/reorder/folders, M03 formats, M04 descriptions |
| Calculations | C01 core expressions, C02 aggregations, C03 parameters, C04 inline type errors |
| Charts | V01 bar, V02 line/area, V03 scatter, V04 table, V05 shelves, V06 axes/legends/tooltips, V07 sort/limit, V08 heatmap, V09 pie, V10 KPI, V11 palettes, V12 suggestion |
| Worksheets | T01 tabs, T03 used-in, T04 fork, T05 delete safety, T06 jump to source |
| Reports | R01 canvas, R02 live references, R03 layout, R04 text, R05 overrides, R06 cross-filter |
| Data out | X01 CSV/Parquet export |
| Automation | A01 CLI, A02 CLI preview render |
| Format | O01 published spec, O02 JSON Schema, O04 migration policy |
| Platform | U07 themes, U08 onboarding workbook |

## 3. Explicitly not in the MVP

Anything in this list is a deliberate no, not an oversight:

- Accounts, billing, licences, or any network call to us
- Print export, PDF, SVG, HTML — preview image only
- Publishing, embedding, sync, hosted anything
- Undo and redo (U16, first thing in 1.0)
- Window functions and LOD expressions (C05, C06)
- Relationships and joins (M05, M06). MVP is single-table only, plus unions are out too
- Maps, small multiples, dual axis, annotations, conditional formatting
- Web build, MCP server
- Accessibility work beyond keyboard operability and readable contrast
- Localisation beyond keeping strings out of logic
- Free-form report layout; the MVP report is a tiled grid

## 4. The workflow, precisely

This is the loop the MVP must nail. Anything that adds friction here is out of scope.

1. **Open a source.** Drag a file onto the window, or connect to Postgres/MySQL. The data pane
   populates with typed fields, split into dimensions and measures.
2. **Make a chart.** Drag a dimension onto the row shelf and a measure onto the column shelf. A
   bar chart appears immediately, with no dialog in the way.
3. **Refine it.** Change the aggregation from the field's menu. Add a second dimension to get
   grouped bars. Drag a measure onto colour. Change the mark to line. All of it re-renders
   without a button press.
4. **Make more charts.** Each new chart is a new tab. Tabs are the workspace: rename, reorder,
   duplicate, close. Nothing is a modal dialog.
5. **Compose a report.** Switch to the report view, pick worksheets from the panel, drop them on
   the grid, resize, add a title and a text block, choose a theme.
6. **Stay current.** Go back to a worksheet, change its filter. Return to the report: it has
   already changed. No refresh, no re-adding the chart.

Every one of those six steps is covered by an acceptance test below.

## 5. Acceptance criteria

Each criterion is a test, not a wish. Numbers are asserted exactly.

| ID | Given | Then |
|---|---|---|
| MVP-01 | A 500k-row CSV dropped on the window | Data pane shows the correct field count and types within 3 s; no modal blocks the view |
| MVP-02 | A Postgres table connected with a read-only user | Same field list as `psql`'s `\d`; first chart renders within 2 s of the connection |
| MVP-03 | Dimension on rows, measure on columns | Chart matches a hand-written SQL `GROUP BY` for every category, value for value |
| MVP-04 | Aggregation changed from sum to average | Values match `AVG` exactly, including integer columns |
| MVP-05 | A filter applied to a worksheet | All totals equal the SQL result with the same `WHERE` clause |
| MVP-06 | A parameter bound to a filter | Changing the parameter re-renders without a full app refresh |
| MVP-07 | A calculated field that divides by a column containing zeros | The chart renders with the engine's documented null semantics and does not crash; the field editor surfaces the risk before saving |
| MVP-08 | A calculated field with a type error | The error is shown inline at the offending span; the chart is not saved with an invalid expression |
| MVP-09 | A worksheet renamed in its tab | The name updates in the report element panel and in the data pane's used-in list |
| MVP-10 | A worksheet used by two reports, then deleted | Both reports keep a visible placeholder; the delete flow warned first and named both reports |
| MVP-11 | A worksheet's filter changed while a report using it is open | The report re-renders with the new numbers; no manual refresh exists anywhere in the UI |
| MVP-12 | A report element's title overridden | Only that report's copy changes; the worksheet and other reports are untouched |
| MVP-13 | A bar clicked in report chart A | Charts B and C in the same report filter to that category; clicking empty space clears |
| MVP-14 | Report resized from 1400 px to 900 px wide | No element overlaps or is clipped; the grid reflows deterministically |
| MVP-15 | Workbook saved, app quit, workbook reopened | Identical worksheets, reports, layout, tab order, filters and theme |
| MVP-16 | A workbook written by 0.2 opened in 0.3 | It opens through the migration path with identical chart semantics |
| MVP-17 | Any result exported as CSV | Byte-identical to the same query run through DuckDB's CSV writer, row order included |
| MVP-18 | `slickbi-cli render` on a report | Produces the same image as the on-screen report within the documented tolerance |
| MVP-19 | App left running for 8 hours with a 1M-row extract | Resident memory grows less than 10% and no task slows below the interaction target |
| MVP-20 | The published JSON Schema validated against the app's own output | Zero errors; the schema is generated from the same types that serialize |

## 6. Quality bars

| Measure | Target |
|---|---|
| Cold start to interactive | under 2 s, warm page cache |
| Chart render after a shelf change | under 300 ms at 1M rows aggregated |
| Report re-render after a worksheet change | under 500 ms |
| Filter or cross-filter round trip | under 500 ms at p95 |
| Crash-free sessions in a 30-minute scripted run | 100% |
| Panics reachable from user input | none; every error path returns a typed error and a message |
| Memory, 1M-row extract loaded | under 1 GB |
| Binary size, release build | under 120 MB per platform |

## 7. The demo script

This is the release test. It is performed by someone other than the implementer, on a clean
machine, with a stopwatch running.

1. Download the build, open it. The sample workbook is offered, not forced.
2. Drag a provided CSV onto the window. Wait for the data pane.
3. Build a bar chart of revenue by region. Add a filter for one year.
4. Duplicate as a line chart of revenue by month. Fix its title.
5. Build a third chart: top ten products by units, as a horizontal bar.
6. Open a new report, drop all three charts on it, add a title and one paragraph of commentary,
   apply a theme.
7. Change the year filter on the first worksheet. Return to the report and confirm it is current.
8. Save. Quit. Reopen. Confirm everything is as left.
9. Export the underlying result to CSV and open it.

Target: **ten minutes**, no documentation, no dialog that requires reading, no dead end.

## 8. Test requirements

Full strategy in [`TESTING.md`](TESTING.md). MVP-specific requirements:

- Unit tests for every public function in `spec`, `calc`, `model`, `planner`, `engine`,
  `workbook` and `scene`, including all error variants.
- Golden SQL tests for every MVP chart shape, including filters, parameters and sorts.
- Integration tests at: planner→engine, engine→scene, workbook round-trip, station→report
  reference resolution, and the CLI against a real workbook.
- A fixture corpus of at least twenty workbooks, including malformed ones, each with an expected
  outcome (opens, or fails with a specific error type).
- Image comparison between `render-gpu` and the reference renderer for every chart type, using a
  small, documented per-pixel tolerance.
- A UI test for the six workflow steps in section 4, driven through the egui test harness.
- A soak test: one million rows, eight hours, memory bounded, run nightly.

## 9. Risks specific to the MVP

| Risk | Why it matters now | Mitigation |
|---|---|---|
| egui drag and drop on shelves feels bad | The whole workflow is dragging; if this is awkward, the product fails at step two | Prototype the shelf interaction in week one, before the rest of the shell |
| No undo at launch | Users expect it; a mis-drag can destroy work | Make destructive operations confirmable and make everything else cheap to redo; ship undo first in 1.0 |
| Report live-reference semantics confuse users | Deleting or editing a shared worksheet has surprising blast radius | Always show "used in N reports", warn before delete, allow fork in one click |
| Single-table limitation frustrates real datasets | Analysts often need two tables | Be explicit in the UI; allow a SQL-defined source as the escape hatch even if joins are 1.1 |
| Migration path exists but is untested against nothing | 0.3 is the first format that anyone keeps | Freeze the format at 0.3, write the migration harness, and never change a serialized field without a fixture |

## 10. Ship checklist

- [ ] Every MVP acceptance criterion passes on all three platforms
- [ ] The demo script completed in under ten minutes by a non-author
- [ ] Format spec and JSON Schema published
- [ ] Changelog written
- [ ] `FEATURES.md` statuses updated to `done` for every shipped ID
- [ ] `cargo xtask ci` green on the release branch
- [ ] No ignored tests, no warnings, no TODOs without IDs
- [ ] A tagged `0.3.0` on `main`, and `main` merged back into `dev`
