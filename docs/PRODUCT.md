# SlickBI — Product

## What it is

A desktop application for visual data analysis. You point it at files or a database, drag fields
onto shelves, and get charts and dashboards. It runs on your machine, on your data, with no
server required. It is free to use in full. You pay only when you want to take something out of
it that other people will see.

Working name: SlickBI. Trademark search is a pre-launch task.

## How people work

Two surfaces, one loop.

**Worksheets** are single charts. Each one is a tab. You pick a data source, drag fields onto
shelves, choose a mark, and iterate until the chart is right. A worksheet owns its data,
encoding, filters and calculations.

**Reports** are the finished document. You compose a report from the worksheets you have already
built, lay them out, add titles and commentary, set the page, and style it.

Reports hold **references** to worksheets, never copies. Change a worksheet's fields, filters,
sort or chart type and every report that uses it updates. The worksheet shows which reports
depend on it, and reports show where each chart came from. If a user genuinely needs a variant,
they fork the worksheet; the two then evolve independently and the report keeps using whichever
one it references.

The rule that keeps this simple: **worksheets own data, reports own presentation.** A report can
change a chart's title, legend, axes, padding and border. It cannot change what the chart
measures. That single rule is why editing a chart can never silently rewrite a finished report's
numbers, and why reports stay current without anyone pressing refresh.

## Positioning

The incumbents are either expensive or entangled:

| | Problem |
|---|---|
| Tableau | Creator is around $75/user/month, annual only |
| Power BI | Desktop is Windows-only and lives inside an M365 subscription |
| Superset, Metabase | Server-first. You run infrastructure before you see a chart |
| Grafana | Built for time series and operations, not analyst-style exploration |
| Evidence, Rill | Code-first. Great for engineers, wrong shape for a dragger |

The gap: **a desktop-first, local-first authoring tool that a non-technical analyst can use
offline, on any of the three desktop platforms, for free.**

## The paywall in one sentence

**Free is everything on your machine. Pro is anything that leaves it.**

That sentence decides every pricing question. If a feature helps someone analyse their own data,
it is free, forever. If a feature produces an artifact aimed at other people — a PDF, a hosted
dashboard, a sync copy on someone else's server — it may be paid.

## Who it is for

| Persona | Job to be done | What they need from us |
|---|---|---|
| Portfolio builder | Load a CSV, build four charts, export a clean PDF or publish a link for a CV or take-home | Cheap, good-looking output, no Windows requirement, no platform account |
| Small-business analyst | Answer questions from their own Postgres or spreadsheets without standing up a BI platform | A real connector, calculated fields, a dashboard their boss can read |
| Student | Learn visual analysis on real data | Free, offline, no licence server |

## Plans

| Plan | Price | Includes |
|---|---|---|
| **Free** | $0 | Everything local and unlimited: all connectors, all chart types, all calculations, unlimited charts and dashboards, cross-filtering, local saves, raw data export (CSV/Parquet), watermarked 1× PNG preview, CLI and MCP automation |
| **Pro monthly** | $14.99/month | Everything in Free, plus print export (PDF, SVG, PNG at 2–4×), self-contained HTML export, hosted sync, published dashboards and embeds, server-side live data, scheduled refresh |
| **Pro yearly** | $99.99/year | The same, 44% off |
| **Founding lifetime** | $199 once | The same as Pro, permanently, limited to the first 500 buyers in a 30-day launch window |

There are no other tiers. There are no seats, no org plans and no feature matrices by company
size. A person paying $14.99 gets exactly what a person paying $99.99 gets, for longer.

There is no 30-day pass. It was considered and cut: at any sensible price it undercuts the
monthly plan, and at twelve purchases a year it undercuts the annual plan too.

### Free tier, specifically

- No account. Not to download, not to run, not to save. Accounts exist only at the moment of
  purchase, and are needed only for Pro features.
- No row limits, chart limits, dashboard limits, connection limits or export-format limits on
  local work.
- Raw data export is free and always will be. Gating a user's own rows behind a paywall would
  make "own your data" a lie.

### Fair-use caps

These apply to every plan, including Free and lifetime, and exist to keep one account from
becoming a hosting bill. They are printed on the pricing page, not discovered at a wall.

| Resource | Cap |
|---|---|
| Sync storage | 10 GB |
| Published dashboards | 20 |
| Published dashboard views | 50,000 / month |
| Scheduled refreshes | 100 / month |
| Live hosted queries | 100,000 / month |
| Concurrent hosted sessions | 3 |

Caps may be adjusted as real costs become known. Features are never moved from one plan to
another and never removed.

### On the lifetime offer

$199 is roughly thirteen months of monthly revenue. A lifetime buyer who stays five years costs
us five years of sync, compute and bandwidth against a single payment. With the caps above that
is roughly $2–4 per month of service, so a five-year holder is close to break-even and a
ten-year holder is a small loss.

That is acceptable because the offer is **launch capital, not a business model**: 500 units,
30 days, once. It funds the first year of infrastructure and rewards the people who showed up
early. The terms will say, in plain language, that fair-use caps may be adjusted for cost while
features are never removed.

### Discounts

Student and regional pricing change the price only. They never change what a user can do.

## The promises

These go in the README, the website and the terms. They constrain future decisions.

1. Your workbooks open without a subscription, forever.
2. No feature that helps you analyse your data is ever paid.
3. No row caps, chart caps, dashboard caps or connector tiers.
4. If the company disappears, the desktop app keeps working and local exports still work.
5. Nothing is uploaded unless you press a button that says upload. No background sync, no
   silent backups, no anonymous usage data that happens to contain your schema.
6. Deleting a synced or published item deletes it: hard delete within 24 hours, cache purge
   immediately, backups expire within 30 days.
7. Your workbook is a custom `.scz` file, but it is not a secret one: it is a zip of documented
   JSON and Parquet, the format specification and its JSON Schema are published on the website,
   and you can read, diff and archive your own work without our software.

## The open format

The application is closed source. The file format is not.

`.scz` is a zip container holding documented JSON for the model, worksheets, reports and theme,
plus Parquet for any local extracts. There is no proprietary binary blob, no encryption tied to
the app, and no serialization that requires our software to decode.

Published on the website, for every format version:

- the full specification: every field, its type, its meaning and its defaults
- a machine-readable JSON Schema, generated from the same Rust types the app uses, so the schema
  cannot drift from the implementation
- a short changelog of what changed between versions and what the migration does
- two or three example workbooks, small enough to read

This is the trust substitute for not being open source. Someone who has been burned by a closed
BI format will not trust another closed BI format; they will trust one they can open with
`unzip`, read, diff in git and archive for a decade. It also means the format can outlive the
company, which is the only honest answer to "what happens if you disappear".

Custom extension, open specification. Those are not in conflict.

## Sync, storage and encryption

Pro accounts can sync workbooks and their data snapshots, and publish dashboards that stay live
without the desktop app running. That requires us to hold data. The posture is stated honestly
rather than dressed up:

- **In transit:** TLS only.
- **At rest:** per-object envelope encryption. A data key per workspace, wrapped by a master key
  held in a key management service. Disk encryption underneath.
- **Account passwords:** hashed with Argon2id. Never recoverable, only resettable by email.
- **Data source credentials:** encrypted with an AEAD cipher (AES-256-GCM), decrypted in memory
  for the duration of a query, never logged, never written to disk in plaintext.
- **Not zero-knowledge.** We hold the keys. That is what makes password reset and "open my
  workbook on a new laptop" work without handing users a passphrase they will lose.

The public statement, which appears on the pricing page and in the app before the first sync:

> Your data is encrypted at rest and we do not read it. Unlike a zero-knowledge product we hold
> the keys, because that is what lets you reset your password and open your work on another
> machine. If that trade is wrong for you, do not use sync — the desktop app never needs it.

A zero-knowledge mode may be offered later as an opt-in for people who want it, clearly labelled
as unrecoverable. It is not on the roadmap before 2.0.

## Hosting and credentials

Published dashboards must work while the author's laptop is closed. That means the hosted service
queries the user's data source directly, which means it holds encrypted credentials.

Rules that keep this from becoming a typical BI cloud:

- Credentials are scoped to a single data source and encrypted individually.
- The UI recommends, and by default suggests creating, a read-only database user.
- Every query is compiled from a stored chart spec against a stored model. Viewers never submit
  SQL, and no endpoint accepts arbitrary SQL.
- Statement timeouts, row ceilings and per-source concurrency limits are enforced server-side.
- Every decryption of a credential is audited with actor, source, time and result. No admin UI
  reads user data; access requires a logged break-glass procedure.
- A user can revoke a credential, which deletes it and immediately disables every dependant
  published dashboard.

Users who refuse to hand over credentials can run the desktop agent instead, which serves the
same published dashboards from their own machine while it is running. It is an escape hatch, not
the default, because it fails the "portfolio site while I am asleep" test.

## Nothing is hosted in v1

Version 1.0 is local-only: no accounts are required, no service stores anything, and the only
server component is the export renderer used by Pro users. Hosted sync, publishing, embeds and
scheduled refresh arrive in 1.2, after the local product is proven.

## Enforcement, honestly

A paywall in a locally-running application is a convenience and ethics gate, not DRM.

| Threat | Effort | Response |
|---|---|---|
| Screenshot the screen | none | Accepted |
| Patch the official binary | hours | Accepted; the licence forbids redistribution, not curiosity |
| Rebuild a renderer from the scene types | days | Accepted; we sell the last mile, not the geometry |
| Share an account | minutes | Concurrent-session cap; no device fingerprinting |
| Publish something illegal | minutes | Report link, takedown process, account suspension |

What is being sold is the last mile done well: exact fonts, pagination, print resolution, one
click, no badge, and a renderer that is maintained. That is a real product and it does not
require pretending the gate is airtight.

## Competitive note

Tableau Public hosts public dashboards for free, so hosted publishing alone is not a
differentiator for the portfolio segment. What SlickBI offers that it does not:

- unlisted and password-protected publishes, because Tableau Public is public by definition
- self-contained HTML you can host yourself, forever, with no vendor in the path
- the same application offline, on Linux, connected to your own databases
- no forced public gallery attached to your name

## Unit economics

| Item | Value |
|---|---|
| Net per monthly subscriber after payment fees | ~$14.10 |
| Net per annual subscriber | ~$96.50 |
| Hosted service cost per Pro user, typical | $0.50–$2.00 / month |
| Infrastructure floor | $40–120 / month |
| Break-even | ~12–20 subscribers |
| 1,000 monthly actives at 3% conversion | ~$420 / month |
| 10,000 monthly actives at 3% conversion | ~$4,200 / month |

This is a modest, sustainable business rather than a venture outcome, and the plan is built
accordingly: keep free-tier marginal cost at zero, and never take on a hosting obligation that
the paying base cannot cover.

## Not building

| Bucket | Items | Why |
|---|---|---|
| Not in v1 | Hosted storage, sync, publishing, scheduled refresh, any service holding user content | Prove the local product first |
| Not planned | Seats, orgs, SSO, comments, row-level security, public gallery, marketplace | Liability without revenue for a single-tier product |
| Out of scope | ETL, orchestration, warehouse modelling, catalogues, semantic-layer-as-a-service | Different product |
