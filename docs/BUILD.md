# Build and tooling

## Requirements

The edition-2021 workspace requires Rust 1.95 or newer, a native linker, rustfmt and
Clippy. Python tools use only the standard library. Install
[uv](https://docs.astral.sh/uv/getting-started/installation/) and use `uv run` for the
interpreter selected by `.python-version`. The first invocation may download Python.
An existing Python 3.10+ interpreter can also run the scripts directly.

Fetch the locked crates once before working offline:

```sh
cargo fetch --locked
cargo build --workspace --release --locked --offline
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
```

Dependency roles and feature choices are explained in
[the dependency rationale](research/emulator-dependencies.md). The manifests
and lockfile define their versions. Hashing and serialization occur during construction,
inspection, and save/load; ordinary execution allocates nothing.

`--offline` does not install the toolchain or dependencies. No network service, hidden
build download, GitHub Actions workflow or emulator runtime service is required.
Standard `cargo fmt --all --check` works; the check script also invokes rustfmt directly
so it can inspect all source and fixture helper files.

## Development checks

Clone the independent hardware suite once alongside this checkout:

```sh
gh repo clone lumirth/hachiware ../hachiware
```

```sh
uv run tools/check.py --out out/check-1
uv run tools/verify_retail.py --out out/retail-1
```

Each output directory must be new. `check.py` runs formatting, default and trace-feature
tests, Clippy, a release build, host-tool unit tests, independent fixture generation and
fixture execution. Pass `--hachiware PATH` if the suite is elsewhere. Rust builds and
local tests do not require the suite checkout. `verify_retail.py` separately requires
private images and runs full event/state partition comparison, snapshot replay, and the
retail workloads, including persistent writes, power interruption and infrared timeout.
`--quick` selects home, menu and settings; `--list` describes the available cases without
private inputs. Repeat `--case NAME` to select individual scenarios.

The conformance report is `out/check-1/conformance/results.json`. Failed cases retain
their observations and command logs beside it. To investigate one diagnostic, use
hachiware's `--case` selection with those generated fixtures; see [TESTING](TESTING.md).
Child Python commands use the same interpreter as the entry point.

On Windows the check/verification scripts select `hachistep.exe`; direct shell examples
in the docs use Unix executable spelling. Verify builds and behavior on each supported
host.

## Output safety

The CLI always opens outputs with create-new semantics. `--out` creates a new directory
before running. It never writes back to input ROM, EEPROM or sensor files. On a model
fault it reports the failing state and can export that stopped state to the newly
created output directory. Trace I/O failures fail the run rather than silently
certifying a complete trace.

The native `state.bin` captures all hardware domains together. The surrounding output
directory is not a transactional multi-file container. Files are synced, and the run
report is written last, but interruption can leave a partial output directory. Keep
original inputs and treat a directory missing its complete report as an interrupted
export. The core does not write files.

## Features

There is no execution-backend or accuracy feature. `trace` adds bus-observation
callbacks to the same implementation:

```sh
cargo build -p hs-cli --release --offline --features trace
mkdir -p out
./target/release/hachistep run --firmware inputs/pokewalker.bin \
  --eeprom inputs/eeprom.bin --milliseconds 100 --bus-trace \
  --trace out/boot-bus.txt --trace-limit 10000 --out out/bus
```

A trace build is intentionally more expensive. Rebuild without the feature for
performance measurements. The CLI reports truncation; a capped trace is not a complete
hardware history. Do not run long private event-vector tests with bus tracing enabled
unless the resulting memory use is intentional.

## Source archives

Create a source archive from a committed revision with Git:

```sh
mkdir -p out
git archive --format=zip --output=out/hachistep.zip HEAD
```

The archive contains the tracked source at that revision. Private inputs and generated
outputs remain local. Preserve the source commit when distributing an archive.
