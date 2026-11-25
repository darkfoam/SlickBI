# Releases

Each release has one purpose and a set of exit criteria that are checked, not assumed. Feature
IDs refer to [`FEATURES.md`](FEATURES.md).

## Versioning policy

Semantic versioning, with the pre-1.0 phases made explicit:

| Phase | Pattern | Meaning |
|---|---|---|
| Alpha | `0.0.x` | Unstable. Anything may change, including the file format. Builds are for the author, then a small closed group. Not advertised. |
| Beta | `0.x.y` | Public and sellable. Breaking changes bump `y`; new features bump `x`; fixes bump the patch. Data and workbooks are migrated, never discarded. |
| Stable | `X.Y.Z` | Standard semantic versioning. The workbook format is frozen within a major version. |

**`1.0.0` is not scheduled and is not tagged by this plan.** It is earned: the line is cut only
after `0.9.x` has run in public for the stability window below with no open severity-one defects,
every performance budget met, and every migration path from `0.0.1` onward exercised. Until then
the highest version this roadmap produces is `0.9.x`.

Release branches are cut from `dev` as `release/<version>`, smoke-tested on all three platforms,
merged to `main` and tagged there. Tags are never moved.

| Version | Phase | Name | Purpose |
|---|---|---|---|
| 0.0.1 | alpha | Spine | The headless core works end to end |
| 0.0.2 | alpha | First Light | A real chart, saved and reopened |
| 0.0.3 | alpha | Workbench | **MVP.** Analysis and reports are genuinely useful |
| 0.0.4 | alpha | Money | First paid build, first revenue |
| 0.0.5 | alpha | Bridge | External agents can drive SlickBI |
| 0.0.6 | alpha | Everywhere | The same core runs in a browser, on local files |
| 0.1.0 | beta | Launch | Public, paid, signed, credible |
| 0.2.0 | beta | Depth | Real analyst workloads |
| 0.3.0 | beta | Cloud | Sync, publishing and live hosted data |
| 0.4.0 | beta | Scale | Broader sources, maps, office formats |
| 0.5.0 | beta | Extend | Plugins, multiple windows, custom domains |
| 0.9.0 | beta | Stabilisation | No new features; prove it holds |
| 1.0.0 | stable | — | Not scheduled. Cut only when 0.9.x meets the stability bar |

---

## 0.0.1 — Spine (alpha)

**Purpose:** prove the compiler before building a pixel of UI.

**Scope:** `slickbi-spec`, `slickbi-calc`, `slickbi-planner`, `slickbi-engine`,
`slickbi-workbook`, `slickbi-cli` (A01).

**Exit criteria**

- A `.scz` containing a model and one worksheet opens, compiles to SQL and returns Arrow batches
  from DuckDB.
- Golden SQL fixtures exist for at least eight query shapes, each with an expected result set.
- A one-million-row Parquet fixture aggregates in under 300 ms on a laptop, measured in CI.
- `cargo xtask ci` is green on macOS, Windows and Linux.
- Every public function in the five crates has unit tests, including error paths.

**Not in this release:** any window, any rendering, any network.

---

## 0.0.2 — First Light (alpha)

**Purpose:** the smallest thing a person can actually use.

**Scope:** D01–D05, M01–M02, V01–V07, T01, T04, A02, `slickbi-scene`, `slickbi-render-gpu`,
`slickbi-app`.

**Exit criteria**

- Open a CSV by dragging it onto the window; the data pane lists typed fields.
- Build a bar, line, area, scatter and table worksheet, each in its own tab.
- Save, close the app, reopen: identical worksheets and identical view state.
- `slickbi-cli render` writes the same worksheet to an image, and the image-diff harness reports
  parity with the on-screen renderer within tolerance.
- No crash after 30 minutes of ordinary use, including window resize, DPI change and a
  million-row dataset.

**Not in this release:** reports, filters, calculations, dashboards of any kind.

---

## 0.0.3 — Workbench (alpha, MVP)

**Purpose:** the product exists. Someone who is not the author can use it.

**Scope:** D06, D07, D14, D15, D18, D19, M03, M04, C01–C04, V08–V12, V23, T03, T05, T06,
R01–R06, X01, U07, U08, O01, O02, O04.

**Exit criteria**

- The full MVP acceptance list in [`MVP.md`](MVP.md) passes.
- A workbook round-trips through save/load with identical semantics, checked against a fixture
  corpus of at least twenty workbooks.
- The format specification and JSON Schema are published and match the implementation.
- Every integration boundary in [`TESTING.md`](TESTING.md) has a passing test.

**Not in this release:** installers for all three platforms, accounts, billing, print export.

---

## 0.0.4 — Money (alpha)

**Purpose:** first revenue, and a build a stranger can install.

**Scope:** X02–X04, S01–S03, U01–U03, V13.

**Exit criteria**

- A purchase completes end to end in Stripe test mode: checkout, webhook, entitlement token,
  export unlocked, cancellation, portal access, failed-payment grace.
- Print export produces a PDF, an SVG and a 4× PNG from a report, with pinned fonts, on all three
  platforms.
- Free users can export a watermarked 1× PNG and cannot export print formats.
- Installers exist for Windows, macOS and Linux, each installed and smoke-tested on a clean
  machine or VM.
- An export of the same report from all three platforms is byte-comparable within the documented
  rendering tolerance.

**Not in this release:** signing, auto-update, Linux distribution packaging.

---

## 0.0.5 — Bridge (alpha)

**Purpose:** other people's agents and scripts can drive SlickBI, a distribution channel that
costs nothing.

**Scope:** A03, A04, A05.

**Exit criteria**

- An MCP client can list workbooks, describe a model, author a worksheet, run it and render a
  preview without opening the GUI.
- Every MCP tool has a contract test, and the server refuses anything the CLI would refuse.
- The server binds loopback only and requires an explicit opt-in per session.
- Documentation includes a worked example against a common MCP client.

**Not in this release:** any listener that is not loopback.

---

## 0.0.6 — Everywhere (alpha)

**Purpose:** a version for people who cannot install software.

**Scope:** W01, W02, W03.

**Exit criteria**

- The web build opens a local CSV and produces the same scene as desktop for the golden set.
- No request leaves the browser during any free-path operation, verified by inspecting traffic
  across a full session.
- A workbook exported from desktop opens in the browser and vice versa, with identical semantics.
- A 10 MB workbook loads in under 5 seconds on a mid-range laptop.

**Not in this release:** any account, any hosted data, any file larger than the browser can hold.

---

## 0.1.0 — Launch (beta)

**Purpose:** public, paid, signed, credible.

**Scope:** C05, C06, D16, D17, R07–R10, R18, U04–U06, U16, X05, S04, V14, V15, O03, O05.

**Exit criteria**

- Signed and notarised installers on all three platforms; auto-update verified from 0.0.6.
- Undo and redo work across worksheets and reports, including structural changes.
- Window and level-of-detail calculations match their documented semantics on the golden set.
- Self-contained HTML export opens offline with no network and works.
- The founding lifetime offer is live, capped at 500, and the cap is enforced server-side.
- Documentation site is live, including the format specification for every released version.
- A fresh user goes from download to a saved report in under ten minutes without reading docs.

---

## 0.2.0 — Depth (beta)

**Purpose:** the workloads real analysts bring.

**Scope:** A06, C07, D08, D13, M05, M06, R11–R13, T02, V16–V19, U09–U12, U17, O06, X06, X07.

**Exit criteria**

- Relationships across three tables produce correct results, verified against hand-written SQL.
- Batch export renders a 40-page report from a CLI script.
- A screen-reader pass over the main flows, with a data-table fallback for every chart type.
- UI strings externalised; at least one non-English locale shipped as proof.
- A reference reader opens a workbook without linking SlickBI.

---

## 0.3.0 — Cloud (beta)

**Purpose:** the paid features that justify the subscription.

**Scope:** M07, R14–R16, S05–S08, U13, W04, X10–X14.

**Exit criteria**

- Sync round-trips a workbook and its snapshot between two machines, with conflict detection.
- A published report stays live while the author's machine is off.
- Credentials are encrypted at rest, never logged and revocable, with revocation taking effect on
  the next query.
- Deletion is hard: a deleted report is unreachable within 24 hours and absent from backups
  within 30.
- A published page has no cookies, no third-party scripts, and is `noindex`.
- The abuse and takedown procedure is written, dry-run tested and linked from every published page.

---

## 0.4.0 — Scale (beta)

**Purpose:** the sources and formats that larger teams ask for.

**Scope:** D09–D12, M08, R17, S09, V21, V22, W05, X08, X09.

**Exit criteria**

- Each new connector passes the shared connector conformance suite.
- Maps render at 60 fps with a 50k-point dataset.
- Version history restores a workbook to any of its last 30 saved states.
- The desktop agent gateway serves a published report with no credentials stored server-side.

---

## 0.5.0 — Extend (beta)

**Purpose:** the things that make it a platform rather than a tool.

**Scope:** S10, U14, U15, X15.

**Exit criteria**

- Plugins load in a sandbox and cannot block the UI thread or read files they were not granted.
- Zero-knowledge mode is documented, tested against key loss, and clearly separated in the UI.
- Custom domains issue and renew certificates without manual steps.

---

## 0.9.0 — Stabilisation (beta)

**Purpose:** prove the product holds before anything is called stable.

**Scope:** no new features. Fixes, performance, documentation, migration coverage and polish only.

**Exit criteria**

- Thirty days in public with no open severity-one defect.
- Every performance budget met on the reference hardware, reported in CI.
- Every released workbook version from `0.0.1` opens and migrates correctly, proven by fixtures.
- Crash-free sessions above 99.9% across the opt-in crash reports.
- A complete upgrade, downgrade and recovery rehearsal performed on all three platforms.

## 1.0.0 — not scheduled

When `0.9.0` meets the bar above, and not before, `1.0.0` is cut with the workbook format frozen
for the major version. Nothing in this roadmap assumes that date.
