# Example datasets — launch assets

Status: planning. **DS01 is specified**; the other entries are candidates, not scheduled.

These are datasets we generate and publish with the product. They are not product features and do
not belong in [`FEATURES.md`](FEATURES.md). They exist to make the first five minutes concrete, to
exercise the performance budgets, and to give the demo script and screenshots one shared source.

Dataset identifiers use `DSnn` so they are never confused with feature IDs.

## Rules for every dataset

- **Synthetic and deterministic.** A fixed seed, no clocks, no environment-derived UUIDs, no
  network access while generating. Regenerating the CSV produces byte-identical output.
- **Not committed at full size.** The repository holds the generator, the manifest and a tiny
  fixture (about 1,000 rows) for tests. The published artifacts live in object storage with
  SHA-256 checksums.
- **Versioned and immutable.** `v1` never changes; a correction ships as `v2` with a new manifest.
- **One contract test per dataset.** Row count, schema, null counts, known aggregates and
  checksums are asserted, and the CSV, Parquet and SQLite copies must agree.
- **No real people or companies.** No names, emails, addresses, brands, payment data or account
  numbers. Customer identifiers are opaque integers generated from the seed.
- **The index says what it proves.** Every scheduled dataset names the features it demonstrates and
  the budget it must meet.

## The list

| ID | Example | Dataset & description | Requirements (summary) |
|---|---|---|---|
| **DS01** | **Atlas Retail** | Ten million order lines from a fictional global retailer over three years (2023–2025): dates, geography, product hierarchy, channels, segments, money, returns, nulls. Designed to be the headline “10 million rows” download and the main demo/screenshot dataset. | 10,000,000 rows × 22 columns; the schema and distributions in §DS01; deterministic seed; CSV + Parquet + SQLite; data pane < 3 s, first chart < 5 s; published checksums and exact aggregates; no PII |
| DS02 | Household cashflow | Two years of daily balances for three accounts, with deposits and withdrawals. The motivating “bank balance over a month” line/area demo. | Proposed. Roughly 2,000 rows; one row per account per day; date + balance + signed amount; no PII. Demonstrates the continuous time axis. |
| DS03 | Messy bank exports | Six months of transactions exported the way real tools write them: CSV, TSV, XLSX, ODS and Numbers, with `$1,234.56`, `($12.00)`, `MM/DD/YY` dates and one formula column. | Proposed. A small (≈100-row) set per format, derived from synthetic accounts. Demonstrates import formats, currency/date parsing and a spreadsheet formula’s stored result. |
| DS04 | Sensor fleet | Ten million timestamped readings from fifty devices, with temperature, pressure, vibration and a status flag. | Proposed. 50 devices × 200,000 readings; one row per reading; timestamp + device + three measures + status. Demonstrates time axes and heatmaps at scale. |
| DS05 | Multi-table sales | Orders, customers and products as three related tables. | Proposed; blocked on relationships/joins (0.2.0, not the MVP). About 1M orders, 100k customers and 10k products, with keys and cardinality. |

---

## DS01 — Atlas Retail (10 million rows)

**One sentence:** a fictional global retailer’s order lines for three years, large enough to be a
credible performance demo and rich enough to build every MVP chart from one table.

**Why this shape:** one row per order line gives a date for a line/area, two dimensions for a
heatmap, a measure for a KPI, a product hierarchy for tables, a signed measure for returns, and
10M rows for the 300 ms render gate. It is a single table on purpose: the MVP is single-table.

### Shape

- **Rows:** exactly 10,000,000 data rows, one header row.
- **Columns:** 22, in the order below.
- **Encoding:** UTF-8, comma-separated, `\n` line endings, no BOM.
- **Time range:** 2023-01-01 through 2025-12-31, inclusive; every calendar day appears.
- **Money:** US dollars only; two decimal places; no currency column, because a mixed-currency
  column without an exchange rate makes sums meaningless. The app’s number/currency *format* is
  applied in the workbook, not stored in the data.

### Schema and distributions

| # | Column | Type | Null | Values and rules |
|---|---|---|---|---|
| 1 | `line_id` | BIGINT | no | 1…10,000,000, unique, strictly increasing |
| 2 | `order_id` | BIGINT | no | 1…2,500,000; about four lines per order; every order’s lines share `order_date`, `region`, `country`, `city`, `channel`, `segment` and `customer_id` |
| 3 | `order_date` | DATE | no | 2023-01-01…2025-12-31; seasonal weights (Q4 ≈ 32%, Q1 ≈ 21%) |
| 4 | `ship_date` | DATE | 2% | between `order_date` and `order_date + 14`; null for the 2% not shipped |
| 5 | `region` | VARCHAR | no | `North America` 40%, `Europe` 30%, `Asia-Pacific` 20%, `Latin America` 10% |
| 6 | `country` | VARCHAR | no | 12 countries, each mapped to exactly one region |
| 7 | `city` | VARCHAR | no | 60 synthetic city names, each mapped to one country |
| 8 | `category` | VARCHAR | no | `Apparel`, `Electronics`, `Home`, `Outdoors`, `Beauty`, `Office` |
| 9 | `subcategory` | VARCHAR | no | 20 values; every subcategory belongs to exactly one category |
| 10 | `product_id` | INTEGER | no | 1…120; each product has one name, category, subcategory and base price |
| 11 | `product_name` | VARCHAR | no | a synthetic adjective + product noun; no real brand |
| 12 | `channel` | VARCHAR | no | `Web`, `Store`, `Partner`, `Mobile` |
| 13 | `segment` | VARCHAR | no | `Consumer`, `SMB`, `Enterprise` |
| 14 | `quantity` | INTEGER | no | 1…12 |
| 15 | `unit_price` | DOUBLE | no | 1.00…999.99, two decimals, tied to the product |
| 16 | `discount` | DOUBLE | no | 0.00…0.35, two decimals; most lines 0–0.10 |
| 17 | `revenue` | DOUBLE | no | `round(quantity × unit_price × (1 − discount), 2)`; negative when `is_return` |
| 18 | `cost` | DOUBLE | no | `round(quantity × unit_price × category_cost_ratio, 2)`; same sign as revenue |
| 19 | `profit` | DOUBLE | no | `round(revenue − cost, 2)`; a small share of lines is loss-making |
| 20 | `is_return` | BOOLEAN | no | true for 3% of lines; those lines have negative revenue, cost and profit |
| 21 | `rating` | INTEGER | 70% | 1…5; only for some delivered lines; null otherwise |
| 22 | `customer_id` | INTEGER | no | 1…250,000; opaque, synthetic; no name/email/address is ever generated |

**Invariants**

- `revenue = round(quantity × unit_price × (1 − discount), 2)` for every row, before the return
  sign is applied; a return negates `revenue`, `cost` and `profit`.
- `profit = revenue − cost` for every row.
- `cost / revenue` is within 0.55–0.95 for non-returns, per category.
- `ship_date >= order_date`, and `ship_date <= order_date + 14` when present.
- `country` belongs to the row’s `region`; `city` belongs to the row’s `country`;
  `subcategory` belongs to the row’s `category`; `product_id` fixes the product name, category,
  subcategory and base price.
- Every `order_id` has a consistent header (date, geography, channel, segment, customer).
- About 8–15% of total revenue is profit; at least 0.5% of lines are loss-making so conditional
  colour and negative-value charts have something to show.
- No duplicate `line_id`; no duplicated rows.

### Determinism and manifest

- Seed: `0x5EED2026` (or a documented equivalent), one generator stream, no time-of-day.
- Regenerating the **CSV** must be byte-identical. Parquet is value-identical across writers even
  if the bytes differ; the manifest records both the value checksum and the file checksum.
- `atlas-retail-10m.manifest.json` records:
  - dataset `id`, display name, `version` (`v1`), seed and generator version;
  - exact row count, column count, schema with types and nullability;
  - per-column null counts, distinct counts, min/max;
  - totals: revenue, cost, profit, quantity, distinct orders, first/last date;
  - per-region and per-category revenue/profit;
  - SHA-256 and byte size of every artifact.

### Artifacts and delivery

| Artifact | Target size | Purpose |
|---|---|---|
| `atlas-retail-10m-v1.csv` | ≤ 2.0 GB | the headline drag-and-drop file |
| `atlas-retail-10m-v1.csv.gz` | ≤ 600 MB | smaller download; **requires the app to recognize the compound `.csv.gz` extension** before this is published |
| `atlas-retail-10m-v1.parquet` | ≤ 400 MB | fast columnar demo and the performance fixture |
| `atlas-retail-10m-v1.sqlite` | ≤ 1.5 GB | the SQLite connector and live-mode demo |
| `atlas-retail-10m-manifest-v1.json` | < 20 KB | schema, aggregates, checksums |
| `atlas-retail-1k-v1.csv` | < 300 KB | the repository test fixture, never the full artifact |

- The generator ships in the repository (for example an `xtask` subcommand) and is the only way
  the dataset is produced.
- Full artifacts are published to object storage, linked from the launch download page, and never
  committed. The repository keeps the generator, the manifest, the 1k-row fixture and the contract
  test.
- The published zip contains the data, the manifest, a `LICENSE` (CC0-1.0 for the data) and a short
  README with the headline numbers and checksums.

### Performance and acceptance

| Requirement | Budget |
|---|---|
| Data pane shows the 22 fields after opening the CSV | < 3 s |
| First chart (revenue by region) | < 5 s |
| 10M-row aggregation with filters | < 2 s on a current laptop |
| 1M-row interactive chart | < 300 ms (the product gate) |
| Memory while opening and charting | < 1 GB |
| CSV, Parquet and SQLite produce the same field types and totals | exact |

The contract test asserts the row count, 22 columns and types, null counts within ±0.1% of the
manifest, the revenue/cost/profit totals to the cent, at least 0.5% loss-making lines, and the
3% return share. The 1k fixture is the deterministic first 1,000 rows of the full dataset; the
manifest records its exact prefix aggregates, and a second test loads it and checks those without
needing the full download.

### Shipping checklist for DS01

- [ ] The generator command and its seed are committed; regeneration is byte-identical for CSV.
- [ ] The manifest generator and the 1k-row fixture are committed.
- [ ] The app recognizes `.csv.gz` (compound extension) and opens it as CSV, or the gz artifact is
      dropped from the release.
- [ ] CSV, Parquet and SQLite copies are generated and their aggregates agree.
- [ ] The contract test passes against the 1k fixture; the full-artifact check is run once before
      publishing.
- [ ] Object storage, checksums, download page and the dataset's CC0 license are in place.
- [ ] Four screenshots and the demo report are built from the published `v1`.

### Launch use

- Headline: **“10 million rows, 22 columns, 3 years of a fictional global retailer.”**
- Four screenshots: revenue over time (continuous axis), revenue by region (bars), category × month
  heatmap, and profit as a KPI with a filter.
- One report with the four charts and one text block, saved as the demo workbook.
- The demo script names the dataset and the exact chart steps; it never depends on a downloaded
  file being in a particular folder without saying so.
- The dataset is versioned in the download page (`v1`) and the app’s sample/onboarding workbook
  keeps using its small in-memory sample, not the 10M download.

---

## Candidates (not scheduled, no requirements yet)

These are listed so they are not lost. Each becomes a numbered dataset only when it has a purpose,
a schema and an owner.

| Idea | Why it would exist |
|---|---|
| A weekly retail P&L with budget vs actual | Demonstrates variance, reference lines (V17) and KPI tiles for the finance audience |
| A product catalogue with 2–3 related tables | Exercises relationships and joins once M05/M06 land in 0.2.0 |
| Ten years of daily weather for 200 cities | Exercises a very long time axis, missing observations and small multiples (V14) |
| A deliberately broken CSV | Exercises import errors, mixed types, duplicate headers and recovery messages |
| A licensed public dataset (for example city taxi trips) | Only if the license permits redistribution and attribution; needs a legal decision before it becomes DSnn |
