# Connector fixtures

Synthetic, tiny spreadsheets that hold the same three-row transaction table in the formats a small
business actually exports:

| File | What it proves |
|---|---|
| `transactions.csv` | `MM/DD/YY` dates, currency text (`$1,234.56`, `-$4.25`, `($1,200.00)`) |
| `transactions.tsv` | the same table with tab delimiters |
| `transactions.xlsx` | a real date cell, currency number formats, and a formula with its stored result |
| `transactions.ods` | the same, in OpenDocument value types |
| `transactions.numbers` | a Numbers table with text dates, currency cells and a formula |
| `transactions.sqlite` | the same table with a `DATE` declared column and real amounts |
| `transactions.sql` | the SQL dump that builds the SQLite fixture (not imported by the app) |

The test `every_fixture_format_imports_the_same_typed_table` in `src/lib.rs` asserts that every
reader produces identical types and values: integers, text, dates, numbers and the formula result.
The test `every_fixture_format_charts_a_balance_over_time` in `slickbi-app` charts each one.

Nothing here comes from a customer, a bank or a real account. The values are invented.

Regenerate:

```sh
python3 crates/connectors/tests/fixtures/generate.py
cargo run -p slickbi-connectors --example make_numbers -- \
  crates/connectors/tests/fixtures/transactions.numbers
```
