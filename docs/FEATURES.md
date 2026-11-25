# Features

The backlog. Every piece of work in this repository maps to an ID here.

- **Tier:** `F` = free, `P` = Pro. Free means free forever, with no account and no caps.
- **Release:** the version that ships it. See [`RELEASES.md`](RELEASES.md).
- **Status:** `planned` → `in progress` → `done`. A feature is `done` only when its tests and
  documentation are done too.

When a feature ships, change its status and reference the commit that delivered it.

## The workflow these features serve

1. Load data, build a **worksheet** (one chart, one tab), iterate until it is right.
2. Repeat for as many charts as the analysis needs.
3. Compose a **report** from those worksheets, lay it out, style it, add commentary.
4. Edit a worksheet later; every report that uses it updates, because reports reference
   worksheets rather than containing copies.

Worksheets own **data and encoding**. Reports own **presentation**. A report may change a chart's
title, legend, axes, padding and border; it may never change what the chart measures. Anything
that needs different data is a forked worksheet.

## Data sources

| ID | Feature | Tier | Release | Status |
|---|---|---|---|---|
| D01 | CSV and TSV, including drag and drop onto the window, with currency and date formats parsed | F | 0.0.2 | done |
| D02 | Excel workbooks (`.xlsx`, `.xlsm`, `.xlsb`, `.xls`) with sheet selection | F | 0.0.2 | done |
| D03 | Parquet | F | 0.0.2 | done |
| D04 | JSON and newline-delimited JSON | F | 0.0.2 | done |
| D05 | SQLite | F | 0.0.2 | done |
| D06 | PostgreSQL | F | 0.0.3 | done |
| D07 | MySQL and MariaDB | F | 0.0.3 | done |
| D08 | DuckDB database files | F | 0.2.0 | planned |
| D09 | BigQuery | F | 0.4.0 | planned |
| D10 | Snowflake | F | 0.4.0 | planned |
| D11 | ClickHouse | F | 0.4.0 | planned |
| D12 | Microsoft SQL Server | F | 0.4.0 | planned |
| D13 | HTTP and JSON API sources | F | 0.2.0 | planned |
| D14 | Local extract to Parquet for any source | F | 0.0.3 | done |
| D15 | Live query mode against the source | F | 0.0.3 | done |
| D16 | Relink a moved or renamed source | F | 0.1.0 | planned |
| D17 | Refresh source schema and re-read | F | 0.1.0 | planned |
| D18 | OpenDocument spreadsheets (`.ods`) with sheet selection | F | 0.0.3 | done |
| D19 | Apple Numbers documents (`.numbers`) with table selection | F | 0.0.3 | done |

## Semantic model

| ID | Feature | Tier | Release | Status |
|---|---|---|---|---|
| M01 | Automatic typing into dimensions and measures | F | 0.0.2 | done |
| M02 | Rename, hide, reorder and group fields into folders | F | 0.0.2 | done |
| M03 | Type override, number and date formats | F | 0.0.3 | done |
| M04 | Field descriptions | F | 0.0.3 | done |
| M05 | Relationships between tables, with cardinality | F | 0.2.0 | planned |
| M06 | Joins and unions in the data model | F | 0.2.0 | planned |
| M07 | Hierarchies and drill paths | F | 0.3.0 | planned |
| M08 | Reusable field sets shared across workbooks | F | 0.4.0 | planned |

## Calculations

| ID | Feature | Tier | Release | Status |
|---|---|---|---|---|
| C01 | Arithmetic, comparison, `IF`/`CASE`, text and date functions | F | 0.0.3 | done |
| C02 | Aggregations, including distinct count and median | F | 0.0.3 | done |
| C03 | Parameters, usable in calculations and filters | F | 0.0.3 | done |
| C04 | Type checking with inline, spanning error messages | F | 0.0.3 | done |
| C05 | Window and table calculations | F | 0.1.0 | planned |
| C06 | Level-of-detail expressions | F | 0.1.0 | planned |
| C07 | Shared calculation library across a workbook | F | 0.2.0 | planned |

## Charts and worksheets

| ID | Feature | Tier | Release | Status |
|---|---|---|---|---|
| V01 | Bar and column charts | F | 0.0.2 | done |
| V02 | Line and area charts | F | 0.0.2 | done |
| V03 | Scatter plots | F | 0.0.2 | done |
| V04 | Tables | F | 0.0.2 | done |
| V05 | Shelves: row, column, colour, size, text, detail, tooltip | F | 0.0.2 | done |
| V06 | Axis, legend and tooltip configuration | F | 0.0.2 | done |
| V07 | Chart-level sort and row limit | F | 0.0.2 | done |
| V08 | Heatmaps | F | 0.0.3 | done |
| V09 | Pie and donut charts | F | 0.0.3 | done |
| V10 | KPI and text tiles | F | 0.0.3 | done |
| V11 | Colourblind-safe palettes by default | F | 0.0.3 | done |
| V12 | Automatic chart suggestion from the current shelves | F | 0.0.3 | done |
| V13 | Dense marks: one million points at interactive frame rates | F | 0.0.4 | planned |
| V14 | Small multiples | F | 0.1.0 | planned |
| V15 | Dual axis | F | 0.1.0 | planned |
| V16 | Box plots and distribution marks | F | 0.2.0 | planned |
| V17 | Reference lines and bands | F | 0.2.0 | planned |
| V18 | Conditional formatting | F | 0.2.0 | planned |
| V19 | Custom themes | F | 0.2.0 | planned |
| V20 | Annotations on marks | F | 0.3.0 | planned |
| V21 | Choropleth maps | F | 0.4.0 | planned |
| V22 | Tile and point maps | F | 0.4.0 | planned |
| V23 | Continuous time axis for date fields on line, area and point charts | F | 0.0.3 | done |
| T01 | Worksheet tabs: create, rename, duplicate, close, reorder | F | 0.0.2 | done |
| T02 | Worksheet navigator: search, jump, thumbnails | F | 0.2.0 | planned |
| T03 | "Used in" indicator: which reports reference this worksheet | F | 0.0.3 | done |
| T04 | Fork a worksheet into an independent copy | F | 0.0.2 | done |
| T05 | Referential integrity: warn before deleting a referenced worksheet, show a placeholder in reports | F | 0.0.3 | done |
| T06 | Jump from a report element to its source worksheet | F | 0.0.3 | done |

## Reports

| ID | Feature | Tier | Release | Status |
|---|---|---|---|---|
| R01 | Report canvas with a tiled grid | F | 0.0.3 | done |
| R02 | Chart elements that reference worksheets live | F | 0.0.3 | done |
| R03 | Move, resize and reorder elements | F | 0.0.3 | done |
| R04 | Text and markdown elements | F | 0.0.3 | done |
| R05 | Per-element presentation overrides: title, legend, axes, padding, border | F | 0.0.3 | done |
| R06 | Cross-filtering between report elements | F | 0.0.3 | planned |
| R07 | Report-level filters | F | 0.1.0 | planned |
| R08 | Page setup: size, margins, header, footer, page numbers | F | 0.1.0 | planned |
| R09 | Print reports with pagination | F | 0.1.0 | planned |
| R10 | Report themes and typography | F | 0.1.0 | planned |
| R11 | Multiple outputs from one report, such as screen and print variants | F | 0.2.0 | planned |
| R12 | Free placement with snapping, alignment and z-order | F | 0.2.0 | planned |
| R13 | Report templates | F | 0.2.0 | planned |
| R14 | Buttons, tabs and navigation between report sections | F | 0.3.0 | planned |
| R15 | Sections and explicit page breaks | F | 0.3.0 | planned |
| R16 | Annotations and commentary blocks | F | 0.3.0 | planned |
| R17 | Responsive reflow for narrow windows | F | 0.4.0 | planned |
| R18 | Present and fullscreen mode | F | 0.1.0 | planned |

## Export and publishing

| ID | Feature | Tier | Release | Status |
|---|---|---|---|---|
| X01 | Raw data export: CSV and Parquet of any result | F | 0.0.3 | done |
| X02 | Preview PNG at 1× with a watermark | F | 0.0.4 | done |
| X03 | Print-quality PNG (2–4×) and SVG | P | 0.0.4 | done |
| X04 | Paginated PDF | P | 0.0.4 | planned |
| X05 | Self-contained HTML report bundle | P | 0.1.0 | planned |
| X06 | Batch and CLI print export | P | 0.2.0 | planned |
| X07 | Paginated report layout for print | P | 0.2.0 | planned |
| X08 | PowerPoint export | P | 0.4.0 | planned |
| X09 | Excel export with native charts | P | 0.4.0 | planned |
| X10 | Publish an unlisted report link | P | 0.3.0 | planned |
| X11 | Embed snippet for an external site | P | 0.3.0 | planned |
| X12 | Password-protected and expiring links | P | 0.3.0 | planned |
| X13 | Scheduled refresh of published reports | P | 0.3.0 | planned |
| X14 | Live published reports, queried server-side | P | 0.3.0 | planned |
| X15 | Custom domain for published reports | P | 0.5.0 | planned |

## Accounts, billing and hosted services

| ID | Feature | Tier | Release | Status |
|---|---|---|---|---|
| S01 | Account creation, only when purchasing | P | 0.0.4 | planned |
| S02 | Checkout, customer portal and invoices | P | 0.0.4 | planned |
| S03 | Signed entitlement tokens with offline grace | P | 0.0.4 | planned |
| S04 | Founding lifetime purchase, capped at 500 | P | 0.1.0 | planned |
| S05 | Workbook and snapshot sync | P | 0.3.0 | planned |
| S06 | Encrypted credential vault for hosted data sources | P | 0.3.0 | planned |
| S07 | Hard delete with export-before-delete | P | 0.3.0 | planned |
| S08 | Fair-use metering and limits | P | 0.3.0 | planned |
| S09 | Version history and restore | P | 0.4.0 | planned |
| S10 | Zero-knowledge workspace mode | P | 0.5.0 | planned |

## Web application

| ID | Feature | Tier | Release | Status |
|---|---|---|---|---|
| W01 | Web application running entirely on local files | F | 0.0.6 | planned |
| W02 | Browser storage persistence (OPFS) | F | 0.0.6 | planned |
| W03 | Import and export workbooks in the browser | F | 0.0.6 | planned |
| W04 | Hosted live data in the web application | P | 0.3.0 | planned |
| W05 | Desktop agent gateway for users who will not store credentials | P | 0.4.0 | planned |

## Automation

| ID | Feature | Tier | Release | Status |
|---|---|---|---|---|
| A01 | CLI: validate, compile to SQL, query, inspect workbooks | F | 0.0.1 | done |
| A02 | CLI: render a worksheet or report to a preview image | F | 0.0.2 | done |
| A03 | Local MCP server over stdio | F | 0.0.5 | planned |
| A04 | MCP tools: describe the model, author and edit worksheets | F | 0.0.5 | planned |
| A05 | MCP tools: run queries, render previews, read and compose reports | F | 0.0.5 | planned |
| A06 | CLI and MCP print export, requiring a Pro entitlement | P | 0.2.0 | planned |

There is no built-in AI. SlickBI exposes the model, the specification and the query engine to
whatever agent the user already has, through MCP and the CLI. No hosted inference, no prompts
leaving the machine on our behalf.

## Open format

| ID | Feature | Tier | Release | Status |
|---|---|---|---|---|
| O01 | Published `.scz` specification, per format version | F | 0.0.3 | done |
| O02 | JSON Schema generated from the implementation and published | F | 0.0.3 | done |
| O03 | Example workbooks and conformance fixtures | F | 0.1.0 | planned |
| O04 | Versioning and migration policy | F | 0.0.3 | done |
| O05 | Format changelog | F | 0.1.0 | planned |
| O06 | Reference reader: a small sample that opens `.scz` without SlickBI | F | 0.2.0 | planned |

## Platform, UX and delivery

| ID | Feature | Tier | Release | Status |
|---|---|---|---|---|
| U01 | Windows installer and portable build | F | 0.0.4 | planned |
| U02 | macOS universal DMG | F | 0.0.4 | planned |
| U03 | Linux `.deb` and AppImage | F | 0.0.4 | planned |
| U04 | Signed and notarised installers | F | 0.1.0 | planned |
| U05 | Auto-update with signed manifests | F | 0.1.0 | planned |
| U06 | File association for `.scz` | F | 0.1.0 | planned |
| U07 | Dark and light themes | F | 0.0.3 | done |
| U08 | Onboarding sample workbook | F | 0.0.3 | done |
| U09 | Keyboard-complete navigation and screen-reader support | F | 0.2.0 | planned |
| U10 | Accessible data-table fallback for every chart | F | 0.2.0 | planned |
| U11 | Localisation infrastructure | F | 0.2.0 | planned |
| U12 | Opt-in crash reporting, no data content | F | 0.2.0 | planned |
| U13 | Command palette | F | 0.3.0 | planned |
| U14 | Multiple windows | F | 0.5.0 | planned |
| U15 | Plugin API for connectors and custom marks | F | 0.5.0 | planned |
| U16 | Undo and redo across worksheets and reports | F | 0.1.0 | planned |
| U17 | Keyboard shortcuts | F | 0.2.0 | planned |

## Ideas recorded for later (not scheduled)

Recorded so they are not lost. These are deliberately **not** scoped, estimated, or assigned to a
release. They are not part of the MVP.

| ID | Idea | Notes |
|---|---|---|
| I01 | An embedded Python 3 console that can use Pandas or Dask to modify and analyse data: data in, data out, with the result usable in charts. | Local analysis of your own data, so it would be free. A Julia and an R console would be welcome too. Not scheduled; no design decided. |
| I02 | A data-manipulation and editing tab, opened first for a new source (like Tableau's data source view), to modify or enhance imported data before or after import/connection: calculated columns, cleaning, formatting, normalisation, and changing column types. | Local work on your own data, so it would be free. Not scheduled; no design decided. |

## Internal, not a feature

Tracked here only so nobody reinvents them:

- The crates in [`ARCHITECTURE.md`](ARCHITECTURE.md).
- The `xtask` CI harness, golden fixture corpus and image-diff harness.
- Packaging scripts, the release pipeline and the docs site that serves the format specification.
