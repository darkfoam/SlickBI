# Testing

Testing is the main quality control in this project, because there is no QA team and no beta
programme until 0.3. The rule is simple: **if a behaviour matters, a test names it.**

## Principles

1. Test the contract, not the implementation. Assert exact values, exact counts, exact error
   variants.
2. Tests must fail for the right reason before they pass. A bug fix starts with a failing test.
3. Integration tests use real components. DuckDB is real, the filesystem is real, the renderer is
   real. Only the network edge is faked.
4. No test depends on wall-clock timing or execution order.
5. Fixtures are committed, small and named. No test downloads data.
6. No user data, ever, in a fixture. Synthetic only.

## The boundary map

Each row is a place where two subsystems meet. Each gets integration tests.

| Boundary | Tests prove |
|---|---|
| Workbook ↔ spec | A saved workbook reloads with identical semantics; malformed files fail with a specific typed error |
| spec ↔ planner | Every encodable worksheet produces a plan; unsupported combinations are rejected with a typed reason |
| calc ↔ planner | A typed expression lowers to SQL that returns the documented value on the engine |
| planner ↔ engine | Golden SQL runs and returns the expected rows, with correct types and null handling |
| engine ↔ scene | Arrow batches become the expected scales, domains and mark counts |
| scene ↔ render-gpu | Golden scenes rasterise within tolerance |
| scene ↔ export | The same scene produces the same image through both backends within tolerance |
| report ↔ worksheet | A change to a worksheet updates every referencing report; a missing worksheet degrades to a placeholder |
| app ↔ workbook | The six MVP workflow steps produce the expected file |
| cli ↔ core | The CLI performs exactly what the library does, with the same errors |
| mcp ↔ cli | Every MCP tool maps to a CLI operation with identical semantics |
| billing ↔ entitlements | Webhooks produce the right entitlement state, including replay and out-of-order events |
| cloud-vault ↔ cloud-query | Credentials decrypt per query, never appear in logs, and revocation takes effect immediately |

## By layer

**`spec`** — unit tests for serialisation round-trips, unknown-field rejection, id validation,
version refusal, and every migration path. A test asserts that the generated JSON Schema accepts
the app's own serialised output.

**`calc`** — unit tests for lexing, parsing, precedence and typing; property tests that parse →
print → parse is stable; table-driven tests that every function lowers to the intended SQL and
evaluates to the documented value on DuckDB, including nulls and type coercion.

**`model`** — unit tests for field resolution, ambiguity errors, role inference and naming.

**`planner`** — golden files: one directory per dialect, one file per query shape, each with the
SQL and an accompanying expected result set. Adding a shape means adding both. No inline SQL
strings in tests.

**`engine`** — integration tests against a real DuckDB over committed Parquet fixtures. Cancellation
is tested by starting a deliberately long query and asserting the interrupt is observed and the
channel reports `Cancelled`, not `Failed`.

**`scene`** — golden geometry for scales, axes and mark positions; property tests that domains
cover all data and that no mark falls outside its panel; a test that a report's element frames never
overlap after a resize.

**`workbook`** — round-trip tests across a fixture corpus of at least twenty files, including
corrupt, truncated, future-version and adversarially large inputs. Each has a known outcome.

**`app`** — UI tests through the egui harness for drag-and-drop onto shelves, tab creation and
reordering, report composition and cross-filter clearing. Plus a state-machine test for the
dirty/revision/cancel behaviour that drives rebuilding.

**Cloud services** — contract tests against the service handlers with a real database, and recorded
webhook fixtures for Stripe replay, duplicate and out-of-order delivery.

## Golden files

- Live in `crates/<crate>/tests/golden/<case>/`.
- Generated once, reviewed by a human, committed.
- Never regenerated as a side effect of running tests. Regeneration is an explicit command that
  prints a diff.
- A change to a golden file is a behaviour change and must appear in the commit message.

## Image comparison

- Every chart type has a reference image, produced by the export renderer.
- The GPU renderer's output is compared against it with a documented per-pixel tolerance and a
  maximum fraction of differing pixels.
- Failures write both images to `target/image-diff/` and print the path.
- Fonts are pinned in the repository so results do not depend on the host's installed fonts.

## Performance

- Benchmarks with a fixed corpus: 1k, 100k and 1M rows.
- Budgets from [`MVP.md`](MVP.md) are asserted, not merely reported. A regression over budget fails
  CI.
- A nightly soak runs eight hours with a 1M-row extract and asserts memory growth below 10%.

## CI gates

`cargo xtask ci` runs, in order, and stops at the first failure:

1. `cargo fmt --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test --workspace` (unit and integration)
4. Golden SQL and image comparison
5. `cargo build --target wasm32-unknown-unknown -p slickbi-wasm`
6. Dependency audit and licence check
7. Crate boundary check (the rules in `ARCHITECTURE.md`)
8. Binary size budget check

Platform matrix: macOS (arm64, x86_64), Windows (x86_64), Linux (x86_64). WASM on Linux only.

## Test data

- Synthetic only: generated CSVs, small Parquet files, a seeded SQLite database.
- One "realistic" fixture of about 50k rows to catch type and encoding problems.
- No fixture may contain a name, email, address or any other personal data, even fake-looking data
  that could be mistaken for real.
