# SlickBI

A desktop application for visual data analysis on Windows, macOS and Linux.

Connect a file or a database. Drag fields onto shelves. Build a chart, then another, each on its
own tab. Compose the charts into a styled report that stays current when the charts change. Save
everything to your own disk.

## Status

Pre-alpha. The repository currently contains the specification and the beginning of the core
libraries. Nothing is installable yet. The MVP target is 0.0.3 — see [`docs/MVP.md`](docs/MVP.md)
and [`docs/RELEASES.md`](docs/RELEASES.md).

## What is free

Everything that happens on your machine, forever, with no account: all connectors, all charts,
all calculations, unlimited worksheets and reports, raw data export, and automation through the
CLI and an MCP server.

Paid is only for what leaves your machine: print-quality export, self-contained HTML, hosted sync,
and published reports. See [`docs/PRODUCT.md`](docs/PRODUCT.md).

## Documentation

| Document | Contents |
|---|---|
| [`docs/PRODUCT.md`](docs/PRODUCT.md) | Positioning, plans, promises, encryption posture |
| [`docs/FEATURES.md`](docs/FEATURES.md) | The backlog, by feature ID |
| [`docs/RELEASES.md`](docs/RELEASES.md) | Release plan and exit criteria |
| [`docs/MVP.md`](docs/MVP.md) | MVP boundary, acceptance criteria, demo script |
| [`docs/EXAMPLE_DATASETS.md`](docs/EXAMPLE_DATASETS.md) | Launch example datasets and their requirements |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | System design |
| [`docs/TESTING.md`](docs/TESTING.md) | Test strategy and CI gates |
| [`AGENTS.md`](AGENTS.md) | How work happens in this repository |

## Building

Requires a current stable Rust toolchain and a C++ toolchain for the embedded database engine.

```
cargo xtask ci      # format, lint, test, boundary checks
```

Platform installers are not wired up yet; that lands in 0.0.4.

## Licence

Proprietary. Free to download and use under the terms in [`LICENSE.md`](LICENSE.md). The workbook
format is documented publicly and is not encumbered by the application licence.
