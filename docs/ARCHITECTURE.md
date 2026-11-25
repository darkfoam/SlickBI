# Architecture

Prose, deliberately. There is no code in this document; it describes responsibilities, boundaries
and flows. When this document and the code disagree, one of them is a defect.

## Principles

1. **One document, two surfaces.** Worksheets hold data and encoding. Reports hold presentation
   and reference worksheets. The workbook is the only source of truth.
2. **The scene is the contract.** Screen and export both consume the same scene graph, so what
   you see is what you export.
3. **Pure functions where possible.** The planner is a pure function of model, worksheet and
   filter state. Everything expensive and testable stays out of the UI.
4. **No hidden network.** The application makes no request the user did not cause.
5. **Errors are types, not strings.** No behaviour is selected by matching on an error message.

## Components

```
┌────────────────────────────────────────────────────────────┐
│ app   winit + egui                                          │
│  data pane · shelves · worksheet tabs · report canvas       │
├────────────────────────────────────────────────────────────┤
│ scene   marks, scales, axes, legends, layout, hit index     │
├────────────────────────────────────────────────────────────┤
│ planner  worksheet + model + filter state → SQL             │
│ calc     expression language → typed AST → SQL              │
│ model    tables, fields, roles, relationships               │
├────────────────────────────────────────────────────────────┤
│ engine     DuckDB · Arrow · cancellation · extracts          │
│ connectors files, SQLite, Postgres, MySQL, …                │
├────────────────────────────────────────────────────────────┤
│ workbook   .scz read/write, migrations, fixtures            │
└────────────────────────────────────────────────────────────┘

Backends of the scene:
  render-gpu    on-screen (wgpu)
  render-view   watermarked 1× preview, free
  export-pro    print SVG/PDF/HTML, paid, shipped separately
```

The arrow only ever points one way. `scene` knows nothing about SQL; `planner` knows nothing
about pixels; `engine` knows nothing about charts.

## Crates

| Crate | Owns | May depend on |
|---|---|---|
| `slickbi-spec` | The document model: workbook, model, worksheet, report, ids, versions | serde, schemars |
| `slickbi-calc` | Expression parsing, typing and SQL lowering | spec |
| `slickbi-model` | Resolution of fields, roles, relationships, naming | spec |
| `slickbi-planner` | Logical plan and SQL emission per dialect | spec, model, calc |
| `slickbi-engine` | DuckDB session, Arrow results, cancellation, extract files | spec, arrow, duckdb |
| `slickbi-connectors` | Source adapters and schema discovery | engine, arrow |
| `slickbi-scene` | Scales, layout, marks, axes, legends, spatial index | spec, arrow |
| `slickbi-render-view` | CPU preview renderer (watermarked, 1×) | scene |
| `slickbi-render-gpu` | wgpu renderer for the scene | scene, wgpu, vello, parley |
| `slickbi-workbook` | `.scz` IO, migrations, fixture loading | spec |
| `slickbi-app` | Shell, panes, interaction, state machines | all of the above except services |
| `slickbi-cli` | Headless operations over the same core | core crates |
| `slickbi-mcp` | Local MCP server over stdio | cli, core crates |
| `slickbi-wasm` | Browser entry point | app, core crates, duckdb-wasm |
| `xtask` | CI and packaging harness | none |

Services are separate binaries and are never linked into the desktop application:

| Service | Owns |
|---|---|
| `cloud-accounts` | Sign-up at purchase, sessions, password reset, deletion |
| `cloud-billing` | Stripe checkout, portal, webhooks, entitlements |
| `cloud-sync` | Workbook and snapshot storage, conflict detection |
| `cloud-publish` | Published reports, embeds, static assets, revocation |
| `cloud-query` | Server-side execution against user sources, per-tenant limits |
| `cloud-vault` | Encrypted data source credentials, rotation, audit |
| `cloud-worker` | Scheduled refresh, deletion jobs, metering rollups |

Boundaries enforced by CI:

- Nothing below `scene` depends on `wgpu`, `winit` or `egui`.
- `spec`, `calc`, `model` and `planner` do not depend on `engine`.
- `app` does not name `duckdb` anywhere; it goes through `engine`.
- Services do not appear in the desktop dependency graph.

## The document model

A workbook contains:

- **Connections.** Where data comes from. Never contains a password or key.
- **Models.** Tables, columns, fields, types, roles and, later, relationships.
- **Worksheets.** One chart each. Encoding, transforms, filters, sort, limit, mark.
- **Reports.** Canvas size, theme, pages, elements. A chart element holds a worksheet id and a
  set of presentation overrides.
- **Parameters and theme.**

The reference rule is enforced in the type system, not in the UI:

- A report element may address only presentation properties of its chart: title, legend, axes,
  padding, border, background.
- Fields, aggregations, filters and marks belong to the worksheet and cannot be set from a report.
- Resolution happens once, when a report is rendered: worksheet id → worksheet → model → plan.
  A missing worksheet resolves to a placeholder element, not an error state that destroys the
  report.

## Data flow

**Authoring a worksheet**

1. The UI mutates the worksheet in the open workbook.
2. The worksheet is planned: resolve fields, expand calculations, derive grouping from encodings,
   apply transforms and filters, order, limit.
3. The planner emits SQL for the DuckDB dialect, with parameters bound.
4. The engine executes it on its own thread and streams Arrow batches back.
5. The scene builder turns batches into scales, positions and marks.
6. The GPU backend draws the scene. The hit index is built alongside it.

Steps 2–5 run off the UI thread, coalesced: rapid edits cancel the previous plan and query.

**Reading a source**

CSV, Parquet and JSON are registered as views over the file, so every query reads its current
contents. A CSV's text columns that hold money (`$1,234.56`, `-$7.77`, `($1.23)`) are selected
through an expression that strips the formatting, and a file whose dates are written month/day/year
is read with that date format, so the model sees numbers and dates rather than text.

Excel, OpenDocument and Apple Numbers have no DuckDB scanner without an extension. Their first
sheet (or the sheet the user picks) is read through a native Rust parser, and the rows are
registered as a typed table: numbers, text, booleans, real dates and timestamps. A formula cell
contributes the value the spreadsheet stored beside the formula, which is the result the user sees.
Database sources are read the same way through their native driver.

**Rendering a report**

1. For each element referencing a worksheet: reuse the worksheet's cached scene if its inputs are
   unchanged, otherwise plan and build.
2. Compose: place scenes into element frames, apply overrides, draw text, background and theme.
3. Cross-filter state is applied as an additional predicate layer per element, compiled into the
   same plan path. There is no separate "filtered query" implementation.

**Editing a worksheet that a report uses**

The report subscribes to a workspace-level revision counter. A worksheet mutation bumps the
revision, marks dependants dirty, and schedules one rebuild on the next frame. Reports never cache
across a revision bump.

## Threading

| Thread | Owns |
|---|---|
| UI | egui state, input, scene assembly on the UI thread when the scene is small |
| Engine | The single DuckDB connection and the query queue |
| Scene | Layout and scale building for large results, using a bounded pool |
| Render | wgpu device, queue and surface |

The engine thread is single because a DuckDB connection is not `Sync`. Every submitted query
carries a cancellation handle; superseded queries are interrupted rather than abandoned.

## Workbook format

`.scz` is a zip container. The document is a single versioned JSON file; extracts ride alongside
it so a workbook reopens without its original sources.

```
workbook.json        the document: format version, id, meta, connections,
                     models, worksheets, reports and theme
data/<table>.parquet optional extracts, one per table, named after the table
thumbs/<id>.png      optional previews (written from 0.0.2)
```

The document is serialised and validated by `slickbi-spec`; `slickbi-workbook` only packages and
unpacks it.

Properties that are not negotiable:

- The format is documented publicly, with a JSON Schema generated from the implementation, and a
  changelog per version (O01, O02, O05).
- Opening a newer format fails cleanly with a typed error. Partial loading never happens.
- Every migration is a pure function from the previous version, covered by a fixture.
- A directory form with the same contents is valid input, so workbooks can live in git.
- No secret is ever written into a workbook.

## Rendering

- The scene is resolution-independent, in points. Backends apply DPI.
- Dense marks use instanced pipelines; vector and text go through vello. Text is shaped once, with
  parley, and the shaped run is stored in the scene so every backend agrees on line breaks.
- Hit testing is a CPU spatial index over scene primitives with datum references, which keeps it
  testable without a GPU.
- Aggregation, binning and downsampling happen in SQL, before the scene. The renderer is never the
  place where a million rows become a hundred points.
- The export backend and the GPU backend are compared by an image-diff test on every chart type.
  Drift is a build failure.

## Engine

- DuckDB 2.0, embedded, built from the bundled amalgamation with the `parquet` and `json` readers
  statically linked; the `duckdb` crate re-exports `arrow` so the engine, scene and connectors
  share one Arrow version.
- Extensions are not auto-installed or auto-loaded (`autoinstall_known_extensions` and
  `autoload_known_extensions` are both set off at open). Nothing reaches the network by default;
  the HTTP client in the dependency tree is a build-time-only dependency of `libduckdb-sys`.
- Arrow is the only interchange format between engine, scene and connectors.
- Extracts are Parquet files inside the workbook, created on demand.
- Cancellation is an `InterruptHandle` obtained per connection; an interrupted query fails with an
  error, and a superseded worksheet query is interrupted rather than abandoned.
- The engine interface is deliberately narrow — open, register a source, execute SQL with bound
  values, interrupt — so a different engine could be substituted without touching the planner.

## Storage and secrets

| Item | Where |
|---|---|
| Workbooks and extracts | User's filesystem, wherever they choose |
| Data source credentials | OS keychain on desktop; encrypted vault on the hosted service |
| Account password | Argon2id hash on the server |
| Entitlement token | Short-lived signed token, stored locally, refreshed on launch |
| Session | Server-side, revocable |

Nothing in the desktop app writes to a hidden application directory except preferences, logs the
user asked for, and the crash dump they opted into.

## Hosted services (0.3.0 and later)

- Static published bundles are served by `cloud-publish` from an internal blob abstraction. There
  is no CDN in v1; the abstraction exists so one can be added without touching call sites.
- Live published reports are queried by `cloud-query`, which holds no credentials of its own and
  asks `cloud-vault` to decrypt per query, in memory, for the duration of that query.
- Every query is compiled from a stored worksheet and a stored model. No endpoint accepts SQL.
- Per-tenant limits: statement timeout, row ceiling, concurrent connections, monthly query count.
- `cloud-worker` performs scheduled refresh and deletion. Deletion is a job with a receipt, so
  "deleted" means "verified deleted", not "marked deleted".
- Users who will not hand over credentials can point a published report at their own desktop
  agent instead. The agent serves the same compiled requests over a loopback listener and is
  never required.

## Automation

- The CLI and the MCP server are thin wrappers over the same core. Neither has behaviour the
  other lacks.
- The MCP server speaks stdio only, binds nothing, and requires a per-session opt-in.
- Print export through automation requires a valid Pro entitlement, checked the same way as the
  GUI. Data export through automation is free, as it is in the GUI.

## Packaging

| Platform | Artifact | Signing |
|---|---|---|
| Windows | MSI and portable zip | Authenticode, HSM-backed certificate |
| macOS | Universal DMG | Developer ID, notarised, hardened runtime |
| Linux | `.deb`, AppImage, AUR package | Repository signing for the AUR-independent formats |

Built per platform on native runners; the engine's C++ dependency makes cross-compilation a
worse trade than three runners.
