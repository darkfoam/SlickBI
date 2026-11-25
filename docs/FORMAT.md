# The `.scz` workbook format

Version 1. This document is the public specification for the workbook container and the JSON
document inside it. It is published so that a workbook can be read, diffed and archived without
SlickBI.

The machine-readable definition is [`workbook.schema.json`](workbook.schema.json), generated from
the same Rust types the application serialises. For the exact name, type and default of every
field, the schema is authoritative; this document explains the shape, the meaning and the rules.

## Container

A `.scz` is a zip archive. No proprietary binary blob, no encryption tied to the application.

```
workbook.json        the document (see below)
data/<table>.parquet optional extracts, one per table, named after the table
thumbs/<id>.png      optional previews, ignored when unknown
```

- `workbook.json` is required and is the single source of truth.
- `data/` entries are read-only snapshots of source data, so a workbook reopens without its
  original files. The name is the table's SQL name; plan on `data/` holding one Parquet file per
  table that was extracted.
- Any other entry is ignored, so a future version can add thumbnails or other sidecars without
  breaking older readers.
- A directory with the same contents is valid input, so workbooks can live in git.

Nothing in the archive is a secret. Passwords, keys and tokens never appear in a workbook.

## Versioning

`workbook.json` carries a top-level `format_version` integer. The only version this build reads and
writes is **1**.

- A document with a **newer** version is refused before any other field is read, with a typed
  "unsupported version" error. It is never partially loaded.
- A document with a missing or non-integer `format_version` is refused with its own error.

### Migration policy

1. The format version is bumped only together with a migration step in `slickbi-spec` and a
   committed fixture of the previous version.
2. Each migration is a pure function from one version's JSON to the next; it never needs the
   application, an engine or a display.
3. Opening a previous version runs the chain of migrations and must produce identical semantics for
   the document's meaning (the fields may gain defaults; a chart must measure the same thing).
4. The published schema is kept per version; the changelog records what changed and what the
   migration did.

The current release has only version 1, so the migration chain is the identity. Adding version 2
means: add the step, add the v1 fixture, add the schema, update this document and the changelog.

## The document

A `Workbook` has these top-level fields:

| Field | Type | Notes |
|---|---|---|
| `format_version` | integer | `1` for this document. |
| `id` | string | Workbook identifier. |
| `meta` | `WorkbookMeta` | `name` (required), `description`, `created`, `modified`, `app_version`. Timestamps are informational and may be absent. |
| `connections` | array of `Connection` | Data sources. Never a secret. |
| `models` | array of `DataModel` | Tables, columns, fields, relationships. |
| `worksheets` | array of `Worksheet` | One chart per tab. |
| `reports` | array of `Report` | Composed documents that reference worksheets. |
| `theme` | `Theme` or absent | Default palette, background, foreground, font. |

Inside a workbook, every object is addressed by a stable string identifier (`WorksheetId`,
`FieldId`, …). Reports reference worksheets by id; worksheets reference models and fields by id.
Identifiers are unique within their kind.

### The semantic model

A `DataModel` holds `tables` (each with `columns`, a name, a classified type and nullability) and
`fields`. A `Field` exposes either a column or a calculated expression:

- `source` is tagged: `{"kind":"column","column":"…"}` or
  `{"kind":"expression","expression":"…"}`. The expression language is owned by `slickbi-calc`.
- `role` is `dimension` or `measure`; `data_type` is one of `integer`, `number`, `text`, `boolean`,
  `date`, `timestamp`, `unknown`.
- `format` optionally describes display (`number` with decimals/thousands/currency, `date` with a
  pattern, or `text` with a character cap).
- `hidden` and `description` are presentation metadata; neither changes a query.

### Worksheets

A `Worksheet` owns what a chart **measures** and is never edited by a report:

- `mark` selects the shape: `bar`, `line`, `area`, `point`, `rect`, `text`, `arc`, `donut`.
- `encoding` holds the shelves: `x`, `y`, `x2`, `y2`, `color`, `size`, `opacity`, `shape`, `text`,
  `detail[]`, `tooltip[]`. Each non-empty shelf carries a field id, an aggregation and optional
  title/scale. Every non-aggregated shelf becomes part of the `GROUP BY`.
- `filters` are predicates applied before aggregation.
- `transforms` are steps between reading and drawing (`bin`, `aggregate`, `calculate`, `window`,
  `sort`, `limit`).
- `sort` and `limit` order and cap the result.

### Reports

A `Report` owns **presentation only**:

- `canvas` (width and height), `target` (`screen` or `print` with page setup), `theme`, `elements`.
- A chart element holds a `worksheet` id and presentation `overrides` (`show_title`, `show_legend`,
  `show_axes`, `padding`, `border`, `background`) plus an optional title. It can never change the
  fields, aggregation or filters; a report that needs different data references a forked worksheet.
- A text element holds markdown.

Deleting a worksheet that a report references leaves the reference in place; the report renders a
placeholder. A missing worksheet is not a malformed document.

## Validation

On load, `slickbi-spec` checks structural invariants: non-empty names, unique identifiers, every
worksheet's model and fields exist, every report element has an area, and a chart's fields belong to
its model. Validation is separate from parsing so an in-memory draft may be invalid without being
unrepresentable.

## Schema

`docs/workbook.schema.json` is generated from the implementation and committed. Two tests keep it
honest:

- the committed file equals the freshly generated schema (so it cannot drift), and
- a representative workbook, serialised by the application, validates against that schema with
  zero errors (MVP-20).

`cargo run -p slickbi-cli -- schema` prints the current schema.

## Changelog

- **1** — first public format: workbook metadata, connections, models (single table and
  relationships), worksheets, reports and theme.
