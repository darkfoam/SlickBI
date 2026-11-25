# Example datasets — launch suite

Status: planning. Fifteen examples are specified. Generators, data files, playable workbooks and
the gallery are to be built.

These are datasets and example workbooks we publish with the product. They are not product
features and do not belong in [`FEATURES.md`](FEATURES.md). They exist to make the first five
minutes concrete, to exercise the performance budgets, and to give the demo, screenshots and
launch gallery one shared source.

Dataset identifiers use `DSnn` so they are never confused with feature IDs.

## The ladder

The suite runs from one person’s money to a Fortune 100 research programme. Every example ships
**data plus a self-contained, playable workbook** with charts, reports, a story and a “try this”
guide.

| Tier | IDs | Audience | Scale |
|---|---|---|---|
| **Tiny** | DS01–DS04 | An individual; a first-time user; an import check | 500–5,000 rows |
| **Small** | DS05–DS08 | A founder, a small shop, a marketing lead, a support lead | 150k–500k rows |
| **Medium** | DS09–DS11 | An operations team, a sales organisation, a multi-table analyst | 2M–5M rows; one multi-table model |
| **Extreme** | DS12–DS15 | A Fortune 100 consumer-research team, a hospital board, a logistics team, an airline | 8M–15M rows; 30–45 columns |

**The 0.0.3 shape rule.** A worksheet reads one table. Examples are single-table unless listed as
multi-table; no example may require a join until relationships land in 0.2.0. Where a story really
wants several tables (DS11), the 0.0.3 build ships a denormalised preview and the normalised
version is the 0.2.0 flagship.

## Generating, storing and showing progress

Examples are generated once and kept on disk. Generation, the first read of a large file and the
first query are all long-running actions, and the app must treat them as such rather than freezing
a window with no explanation.

- **Choose where to store.** The first time an example needs generated data, prompt for a folder,
  show the approximate size, and offer a sensible default (the app’s data folder or
  `<Documents>/SlickBI Examples`). Remember the choice per installation and per example. Never
  write outside the chosen folder without consent; show the path and offer “Open folder”.
- **Generate once, read later.** Write the data (CSV/Parquet/SQLite), the manifest and the
  checksums to the chosen folder. Opening the example again reads the saved files; it does not
  regenerate. If a checksum or the manifest version does not match, say so and offer
  “Regenerate”. Provide “Delete generated data” as well.
- **Atomic writes.** Write to a temporary file and rename only when the file is complete and its
  checksum passed. A failed generation must never leave a partial file that looks usable. Check
  free disk space before starting.
- **A live progress window.** Any action that can take more than a moment — generating rows,
  writing a file, reading a large file, planning or running the first query, rendering a report —
  shows a non-modal status window with a specific, updating message:
  - the stage (“Preparing”, “Generating rows”, “Writing Parquet”, “Reading the file”, “Planning
    the chart”, “Querying”, “Rendering the report”);
  - current and total rows or bytes, elapsed time, and the destination path where relevant;
  - a progress bar when a total is known, and a spinner otherwise;
  - a Cancel button for generation and queries;
  - a success state with the file path and a “Show details” log, or a clear error with the same;
  - updates at least every 200 ms; the window never becomes a frozen “not responding” shell.
- **Responsive by construction.** The work runs off the UI thread (or on a worker with progress
  messages), so a slow example never blocks typing, scrolling or the window itself. A modal is
  reserved for a decision the user must make (such as choosing the folder), never for waiting.
- **One mechanism everywhere.** The same status window covers the onboarding sample, example
  downloads, large CSV imports, first queries and report rendering, so testers always know whether
  the app is working or stuck.

This is a prerequisite for the example gallery and for outside testers; the current app runs these
steps synchronously and reports many failures only to stderr.

## Rules for every dataset

- **Synthetic, but tangible.** Values are generated, never copied. The generator models
  correlations and trends; independent random columns are not enough. Every dataset has
  **planted truths** in its manifest, and the reports reveal them.
- **Deterministic.** Fixed seed, no clocks, no environment-derived UUIDs, no network access while
  generating. Regenerating the CSV is byte-identical.
- **Playable.** A `.scz` with its data embedded as an extract opens offline on a clean machine,
  with no source file and no path to find.
- **Charts and reports included.** Named worksheets, at least one report, a theme, field
  descriptions, number/date formats, folders, a continuous time chart, a category chart, a
  heatmap where the shape allows, a table and a KPI. At least one parameter and one cross-filter
  interaction are part of the example.
- **Not committed at full size.** The repository holds the generator, the manifest and a ~1,000-row
  fixture. Full artifacts and playable workbooks live in object storage with SHA-256 checksums.
- **Versioned and immutable.** `v1` never changes; a correction is `v2` with a new manifest.
- **No real people or companies.** No names, emails, addresses, brands, payment data or account
  numbers. Patient, customer, seller and employee identifiers are opaque integers from the seed;
  all names are generated.
- **One contract test per dataset.** Row count, schema, null counts, known aggregates, planted
  truths and checksums; CSV, Parquet and SQLite agree; the workbook opens and every worksheet and
  report builds.
- **The index says what it proves.** Every dataset names the features it exercises and the budget
  it must meet.

---

## The catalog

| ID | Example | Audience | Rows | Shape | Headline reports | MVP status |
|---|---|---|---|---|---|---|
| **DS01** | Household ledger | One person | ~5k transactions, ~3.6k daily balances | Two companion tables, one workbook | Money this month; Balance over time | **Ready** |
| **DS02** | Messy bank exports | Anyone importing their bank’s files | ~500 per format | CSV, TSV, XLSX, ODS, Numbers, plus broken variants | Import check; Spending by category | **Ready**, error surface needs work |
| **DS03** | Coffee shop | A small shop owner | ~1,000 daily rows | One table | Daily sales; Weekday pattern | **Ready** |
| **DS04** | Fitness & sleep | An individual | ~2,000 days | One table | Sleep & resting heart rate; Weekday heatmap | **Ready** |
| **DS05** | Startup pulse | Founder / ops lead | ~250k events | One event table | Weekly business review; Support & retention | **Ready** |
| **DS06** | E-commerce orders | Small retailer | ~150k order lines | One table | Revenue & margin; Product mix | **Ready** |
| **DS07** | Marketing attribution | Marketing lead | ~500k daily channel rows | One table | Channel ROAS; Campaign heatmap | **Ready** |
| **DS08** | Support desk | Support lead | ~300k tickets | One table | Volume & SLA; Satisfaction | **Ready** |
| **DS09** | Sensor fleet | Operations / IoT team | ~5M readings | One narrow table | Device health; Hourly heatmap | **Ready if aggregated** |
| **DS10** | Corporate sales | Sales ops, regional managers, CFO | ~2M order lines | One table | Quarterly sales review; Regional deep dive | **Ready** |
| **DS11** | Multi-table sales | Analyst team with targets | ~1M orders, 3M lines, 5 tables | Star schema: customers, products, orders, lines, targets | Attainment vs target; Customer & product performance | **Blocked on M05/M06** (0.2.0); denormalised preview playable |
| **DS12** | Fortune 100 consumer research | Corporate board and research team | ~15M consumer signals | One 40-column table now; surveys/media/sales at 0.2.0 | Brand tracker; Campaign & segment deep dive | **Ready denormalised**; no maps/stat tests |
| **DS13** | Hospital operations | Hospital board and analysts | ~10M encounters | One 35-column table now; patients/encounters/labs at 0.2.0 | Admissions & capacity; Readmissions & cost | **Ready denormalised**; no PHI, no clinical claims |
| **DS14** | Global supply chain | Logistics team | ~8M shipment events | One 30-column table | On-time performance; Lane deep dive | **Ready denormalised**; no maps |
| **DS15** | Airline operations | Airline ops / analysts | ~10M flights | One 30-column table | Delay performance; Carrier & airport deep dive | **Ready denormalised**; no maps |

“Ready” means the current MVP can build the playable reports from the single-table shape,
subject to the performance verification in each section. “Blocked” means the example cannot be
played as specified without a scope change.

---

## Tiny

### DS01 — Household ledger

**Audience / story.** A salaried designer’s two years of money across five accounts. “Money this
month” answers where it went; “Balance over time” answers whether the balance survives until
payday. This is the motivating bank-balance story.

**Tables.** `transactions` (5,000 rows, 15 columns: id, account, posted_date, month, merchant,
description, category, subcategory, type, signed amount, spend, income, balance_after, payment
method, recurring) and `daily_balances` (3,650 rows: date, account, balance, inflow, outflow, net,
transaction count). Two models in one workbook; no join.

**Planted truths.** December dining/gifts +35%; the subscription rises from $14.99 to $19.99 in
month 7; one April medical expense; the checking balance dips before every payday.

**Reports.** “Money this month”: spend by category, top merchants, income vs spend by month,
recurring charges, net-cash-flow KPI, a month parameter. “Balance over time”: balance by account
(line), monthly net change (bar), latest/lowest balance KPIs, an account parameter.

**Status.** Ready. **Requires** the continuous time axis, colour series, bar, table, KPI,
parameters, cross-filter and two models in one workbook.

### DS02 — Messy bank exports

**Audience / story.** The same household, exported the way real tools write files: a bank CSV
with `11/21/26` dates, a Google Sheets CSV with `$1,234.56` and `($12.00)`, a TSV, an XLSX, an ODS,
a Numbers document with text dates and a formula, and a deliberately broken CSV with mixed date
formats and an empty row. This is the import-proof example.

**Scale.** ~100–500 rows per file, six or seven files; tiny.

**Requirements.** Every good file imports the same table: dates as dates, currency text as
numbers, the spreadsheet formula’s cached result, and the correct field types. The broken file
must fail with a clear message and must not crash or hang. A multi-sheet workbook must offer the
sheet picker. The example ships an “Import check” report (a table of parsed rows, spend by
category, a row-count KPI).

**Status.** Formats and parsing are ready. The **broken-file error surface** needs verification in
the GUI (today errors go to stderr; testers need a visible message).

### DS03 — Coffee shop

**Audience / story.** A two-person coffee shop tracking daily sales for three years. The first
example a new user opens: one table, one date, one measure.

**Scale.** ~1,000 daily rows. Columns: date, weekday, weather, temperature, product category,
transactions, revenue, cost, staff hours.

**Planted truths.** Weekend mornings are busier; rain reduces walk-in revenue; a price rise in
2025 lifts revenue but not transactions; summer iced drinks change the category mix.

**Reports.** Daily revenue (line), revenue by weekday (bar), weather vs revenue (scatter), cost
and margin KPIs, a small table.

**Status.** Ready. **Requires** the date axis, line, bar, scatter, KPI and table.

### DS04 — Fitness & sleep

**Audience / story.** Two years of one person’s sleep, resting heart rate, steps and workouts.
A gentle personal-health example with no medical claims.

**Scale.** ~2,000 days. Columns: date, sleep_hours, resting_hr, steps, workout_type, duration,
calories, mood.

**Planted truths.** Sleep drops in a stressful work month; resting heart rate rises when sleep is
low; weekend workouts are longer; a holiday week breaks the pattern.

**Reports.** Sleep and resting heart rate (line), weekday × metric heatmap, workout mix (bar),
monthly averages (table), KPI.

**Status.** Ready. **Requires** a precomputed weekday column for the heatmap.

---

## Small

### DS05 — Startup pulse

**Audience / story.** A 12-person SaaS team’s 18 months of events. The weekly review asks what is
growing, which channel is worth paying for, and whether support is keeping up.

**Scale.** ~250,000 events, 16 columns: event id, occurred_at, event_week, user, account, plan,
event, feature, channel, country, device, seats, MRR, response minutes, satisfaction, ticket
priority. `mrr_snapshot` rows make MRR safe to sum; `event_week` makes a heatmap possible without
window functions.

**Planted truths.** Paid-search signups grow but churn at twice the rate after 60 days; a March
support backlog; a June feature launch lifts weekly active users; a September price change raises
MRR and churn.

**Reports.** “Weekly business review”: WAU, signups by channel, MRR by plan, feature × week
heatmap, churn KPI, a plan parameter. “Support & retention”: response time, tickets by priority,
satisfaction, churn by plan/channel, at-risk accounts table, a channel cross-filter.

**Status.** Ready.

### DS06 — E-commerce orders

**Audience / story.** A small online retailer’s two years of order lines: what sells, where, at
what margin, and what comes back.

**Scale.** ~150,000 order lines, 20 columns: order/line ids, date, month, country, channel,
segment, customer id, product id/name, category/subcategory, quantity, price, discount, revenue,
cost, margin, return flag/reason.

**Planted truths.** One category’s returns double in Q4; a discount push raises revenue but cuts
margin; a new market grows; one product is a quiet bestseller.

**Reports.** Revenue & margin over time (line), category bars, country × month heatmap, top
products table, return-rate KPI, a country parameter.

**Status.** Ready.

### DS07 — Marketing attribution

**Audience / story.** A marketing lead’s two years of daily channel and campaign performance.
Which channel returns the spend, and which campaign is buying empty clicks?

**Scale.** ~500,000 rows: date, channel, campaign, region, impressions, clicks, spend, signups,
conversions, new customers, revenue, cost. Calculated fields for CTR, CAC, ROAS and conversion
rate.

**Planted truths.** Paid search has the best last-click ROAS but the highest CAC; a brand campaign
lifts direct traffic; one social campaign has high clicks and almost no conversions; Q4 CPCs rise.

**Reports.** Spend vs revenue (line/bar), ROAS by channel (bar), campaign × month heatmap, CAC KPI,
a channel parameter, a table of campaigns. **No statistical significance claims**; it is
descriptive attribution.

**Status.** Ready; depends on calculated fields (C01/C02).

### DS08 — Support desk

**Audience / story.** A support lead’s year of tickets: are we fast, are we resolving, and are
customers satisfied?

**Scale.** ~300,000 tickets: created/resolved timestamps, priority, team, channel, product,
first-response minutes, resolution hours, satisfaction, reopened flag.

**Planted truths.** A release in March spikes tickets for three weeks; urgent tickets are fast but
poorly rated; one team’s reopen rate is high; chat is faster than email.

**Reports.** Volume and first response (line), priority bars, team × week heatmap, satisfaction
distribution, top reopen table, KPI.

**Status.** Ready.

---

## Medium

### DS09 — Sensor fleet

**Audience / story.** Fifty machines in five sites, one reading per device every few minutes for
a year. An operations team asks which device is drifting, which site is vibrating, and what the
last firmware did.

**Scale.** ~5,000,000 readings, 11 columns: reading_at, device_id, site, line, temperature,
pressure, vibration, humidity, status, firmware, reading_hour (precomputed for a heatmap).

**Planted truths.** One device drifts in pressure from month 8; a site’s vibration spikes after a
firmware update; temperature follows the seasons; a status warning precedes most failures.

**Reports.** Hourly average temperature (line), device × hour heatmap, alert rate by site (bar),
firmware comparison (bar), a device table, a site parameter.

**Status.** Ready **if the line and heatmap use aggregates** (hourly/day). A raw 5M-point chart is
not an MVP shape: dense marks land in 0.0.4 (V13). The example must aggregate.

### DS10 — Corporate sales

**Audience / story.** Three years of order lines for a fictional industrial company: 8 regions,
40 reps, 500 products, 5,000 customers. The quarterly review asks where growth and margin are; the
regional dive asks who is behind and why.

**Scale.** ~2,000,000 lines, 28 columns: ids, dates, month, region/country/city, rep/team,
channel, segment, customer, product/category/subcategory, quantity, price, discount, revenue, cost,
margin, return flag/reason, payment terms. No quota column: a quota is per rep per month and
summing it over lines would be wrong. Targets are the DS11 multi-table companion.

**Planted truths.** Q4 is ~32% of revenue; Europe discounting erodes margin in H2 2024; Enterprise
grows while SMB declines; one category’s returns spike in 2024 Q3; one team outperforms.

**Reports.** “Quarterly sales review”: revenue trend, region bars, category × month heatmap, rep
table, discount vs margin scatter, revenue/margin/return KPIs, a region parameter. “Regional deep
dive”: country bars, category mix, rep table, return reasons, a country cross-filter.

**Status.** Ready.

### DS11 — Multi-table sales (the one scope decision)

**Audience / story.** An analyst team with a proper star schema and sales targets. This is the
example that proves relationships and joins, and the only one that cannot be played as specified
in 0.0.3.

**Scale and tables.**

| Table | Rows | Columns |
|---|---|---|
| `customers` | 100,000 | id, name, segment, country, region, industry, since date |
| `products` | 10,000 | id, name, category, subcategory, list price, cost ratio |
| `orders` | 1,000,000 | id, customer id, order date, channel, rep id, payment terms |
| `order_lines` | 3,000,000 | order id, line id, product id, quantity, unit price, discount, revenue, cost, margin, return flag |
| `sales_targets` | 40 reps × 36 months | rep id, month, target revenue, target margin |

**Planted truths.** One region misses target because of discounting; a rep beats target on a few
large deals; the customer segment mix shifts; a product line’s margin falls; targets are missed in
the quarter after a price rise.

**Reports.** Attainment vs target (bar/line), revenue by segment (bar), top customers and products
(tables), margin by region (heatmap), a rep parameter, a customer cross-filter.

**Status.** **Blocked on M05/M06 (relationships and joins, currently 0.2.0).** A 0.0.3
**denormalised preview** can ship from one order-line table with customer/product attributes (like
DS10) and all charts except attainment vs target; the real five-table model and targets report are
the 0.2.0 flagship.

---

## Extreme

### DS12 — Fortune 100 consumer research

**Audience / story.** A global consumer-goods company’s brand tracker: 15 million consumer signals
across surveys, reviews and social mentions, prepared for a board that asks why a flagship brand
lost consideration in one market while a challenger gained.

**Scale.** ~15,000,000 rows, 40 columns. The 0.0.3 shape is one **consumer-signal** table:

- signal id, signal_date, month, wave, source (survey/review/social), market, region, country;
- respondent/consumer id, segment (New, Returning, Loyal, At risk), age band, income band, gender
  (synthetic), tenure;
- brand, category, subcategory, product, price paid, competitor flag;
- awareness, consideration, purchase intent, NPS (0–10), ad recall, media channel, campaign;
- sentiment (−1…1), review rating, verbatim text, helpful votes, verified flag;
- survey weight, media spend in the market/wave, category sales in the market/month.

**Planted truths.** A Q3 campaign lifts awareness but not consideration; a challenger gains among
under-35s in one market; a price rise lowers NPS in the value segment; the new-customer mix shifts
after a promotion; verbatims cluster around one product complaint.

**Reports.** “Brand tracker”: awareness/consideration/intent by wave and brand (bars/lines), NPS
trend, market × month heatmap, segment comparison, verbatim table, a brand parameter. “Campaign &
segment deep dive”: ad recall vs spend (scatter), segment mix, competitor share, a market
cross-filter. “Board pack”: six charts and a text summary.

**Requirements and MVP fit.** The denormalised table is playable with grouped aggregates, weighted
averages via calculated fields (`sum(measure × weight) / sum(weight)`), heatmaps, scatter, tables
and KPIs. **No maps**, **no statistical-significance tests**, **no regression/forecasting**, **no
text search or NLP**; sentiment must be precomputed. A normalised surveys/media/sales model is a
0.2.0 companion and needs joins.

**Status.** Ready in the denormalised shape; the 15M full file needs performance verification; the
playable workbook uses a 1M sample.

### DS13 — Hospital operations

**Audience / story.** A hospital board and its analysts: ten million encounters over three years.
Why did winter capacity break, why is one department’s wait time high, and did the readmission
programme work? Fully synthetic, no real patients and no clinical claims.

**Scale.** ~10,000,000 encounters, 35 columns: encounter id, synthetic patient id, admit/discharge
dates, month, hospital/site, region, department, service line, synthetic diagnosis group,
severity, age band, synthetic sex, payer, length of stay, ICU flag, readmitted_30d, cost, charged,
staffed beds, occupancy, wait minutes, satisfaction, discharge disposition, primary procedure,
attending team, shift.

**Planted truths.** A winter respiratory surge raises occupancy and waits; one department’s wait
spike follows a staffing gap; the readmission programme lowers readmissions from Q3 2024; cost per
case rises with length of stay; weekend discharges differ.

**Reports.** “Admissions & capacity”: admissions trend, occupancy line, department wait heatmap,
staffing vs wait scatter, KPI. “Readmissions & cost”: readmission rate by department and quarter,
cost vs length of stay, payer mix, a department parameter, a table. “Board pack”: capacity,
readmissions, cost and satisfaction in one page.

**Requirements and MVP fit.** Denormalised playable shape with descriptive aggregates. No
patient-level PHI, no clinical decision claims, no maps, no statistical tests, no forecasting. A
normalised patients/encounters/labs model is a 0.2.0 companion and needs joins.

**Status.** Ready denormalised; full-file performance to verify; 1M playable sample.

### DS14 — Global supply chain

**Audience / story.** A manufacturer’s eight million shipment events: which lanes are late, which
carrier is deteriorating, and what a port congestion costs. The logistics team wants a weekly
exception review.

**Scale.** ~8,000,000 events, 30 columns: shipment id, ship date, month, origin/destination
country, port, lane, carrier, mode (Sea, Air, Road, Rail), weight, volume, cost, promised days,
actual days, delay days, on-time flag, damaged flag, SKU, category, customer segment, warehouse.

**Planted truths.** One port’s congestion adds six days in Q4; a carrier’s on-time rate declines
over two quarters; air freight is used as a costly recovery; one lane’s cost per kg rises; damage
clusters in one warehouse.

**Reports.** On-time trend (line), lane × month delay heatmap, carrier bars, cost vs delay
scatter, exception table, a lane parameter, cross-filter on a carrier.

**Status.** Ready denormalised; **no maps** (use lane/country bars); 1M playable sample.

### DS15 — Airline operations

**Audience / story.** Ten million flights over three years for a fictional airline group: where
delays start, which airports and carriers are worst, and what weather costs. The operations team
runs a daily and a quarterly review.

**Scale.** ~10,000,000 flights, 30 columns: flight id, date, month, carrier, origin, destination,
route, aircraft type, seats, passengers, departure delay, arrival delay, cancelled, weather code,
temperature, wind, distance, fuel, revenue, cost, load factor.

**Planted truths.** A winter storm week cancels one hub’s flights; one carrier’s turnaround
problem raises departure delays; long-haul routes recover delays better than short-haul; a fuel
price rise hits cost per seat; one route’s load factor falls.

**Reports.** Delay trend (line), airport × month delay heatmap, carrier bars, delay vs distance
scatter, route table, a carrier parameter, cross-filter on an airport.

**Status.** Ready denormalised; **no maps**, **no candlestick/forecast**; 1M playable sample.

---

## What the examples require from the MVP

| Capability | Needed by | MVP state |
|---|---|---|
| One-table files and databases; all connectors | All | **Ready** |
| Continuous date axis; line/area/bar/point/heatmap; table/KPI/pie | All | **Ready** |
| Calculations, aggregations, parameters, filters, sort, limit | DS01–DS10, DS12–DS15 | **Ready** |
| Cross-filter in reports | All playable workbooks | **Ready** |
| 10M-row file views and grouped aggregates | DS09, DS12–DS15 | **Ready, needs performance verification** |
| Multi-table relationships and joins | DS11, normalised companions | **Missing (M05/M06, 0.2.0)** |
| Maps | DS12–DS15 if a map is wanted | **Missing (V21/V22, 0.4.0); examples use region bars/heatmaps** |
| Statistical tests, regression, forecasting | DS12–DS15 if claimed | **Not planned; examples stay descriptive** |
| Window/cohort functions | DS12 survey weights/cohorts | **C05, 0.1.0; precompute cohort columns** |
| Dense 1M+ raw marks | DS09 raw readings | **V13, 0.0.4; aggregate instead** |
| Text search / sentiment analysis | DS12, DS13 verbatims | **Not in MVP; sentiment is precomputed, text shown in tables** |
| Broken-file recovery UX | DS02 | **Error is surfaced; GUI error surface/logging needs work** |
| 10M playable workbooks | DS12–DS15 | **Use 1M deterministic samples for the `.scz`; full data separate** |
| Folder prompt + persistent generated-data cache | All | **Not built** |
| Live progress/status window for long actions | All | **Not built** |

### The one hard blocker

**DS11 as a real multi-table example needs M05/M06.** If the MVP must play the five-table model
with targets, relationships/joins have to move from 0.2.0 into 0.0.3. That is a major addition:
relationship resolution, join planning, cardinality, field lineage, the UI and tests. The
alternative is to ship the denormalised preview now and the five-table flagship at 0.2.0. Every
other example is playable in the current single-table MVP with the shapes above.

## Prerequisites before inviting testers

The feature set is complete (51/51), but the examples above do not exist yet and a few
acceptance and distribution gaps matter for outside play.

**Build the suite (the main work).**

1. The 1k fixtures, deterministic generators, manifests and contract tests.
2. The playable `.scz` workbooks and the “try this” guides, starting with DS01–DS08 (the first
   week of play) and then DS09, DS10, DS12–DS15.
3. The 1M samples and full downloads; checksums, licenses and the gallery page.
4. Render every report and capture screenshots at 1400×900 and 900 px.

**Close the MVP gaps that testers will hit.**

- MVP-02: measure the PostgreSQL field list and first-chart budget against a real server.
- MVP-07: warn in the editor when a calculation divides by a value that can be zero.
- MVP-14: a deterministic report reflow test at 1400 and 900 px.
- MVP-16: a migration fixture for an older workbook.
- MVP-17: CSV export through DuckDB’s writer so it is byte-identical.
- MVP-18: an image-diff harness between the app renderer and the CLI report renderer.
- MVP-19: the 8-hour soak and memory-growth test.
- The 1M-row 300 ms chart gate, memory <1 GB and binary <120 MB measurements.
- A ≥20-workbook fixture corpus and the egui UI harness.
- Dependency/licence/wasm/binary-size CI gates.

**Make it installable and reportable.**

- A build testers can install without `cargo`: at least a macOS `.app`/DMG, and Windows/Linux if
  the invite includes them. The 0.0.3 release plan currently says installers are not in this
  release; for an external alpha that has to change.
- The folder prompt, persistent generated-data cache and live progress window described above:
  generated examples are stored on disk for fast reuse, and slow generation/reading/querying
  shows a specific, updating status instead of a frozen window.
- A visible error surface and a “Copy diagnostics” action (version, commit, OS, log path); today
  many errors go to stderr, which a GUI tester never sees.
- A feedback channel and a known-issues page: no undo/redo, no autosave, no PDF/SVG export,
  session-only database credentials, live-mode refusals, multi-table examples at 0.2.0.
- A short “Your first 10 minutes” tour using DS01 and DS03.
- A cross-platform smoke run on the OSes you invite.

**Decide the one scope question.** Denormalised DS11 for the playable alpha (recommended), or move
M05/M06 into the MVP and delay the alpha.

**Then ship 0.0.3.** Changelog, `FEATURES.md`, `release/0.0.3`, `cargo xtask ci`, tag on `main`,
merge back to `dev`.
