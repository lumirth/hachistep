# Build and tooling

## Requirements

The edition-2021 workspace requires Rust 1.95 or newer, a native linker, rustfmt and
Clippy. Python tools use Python 3.10+ and its standard library.

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

## Single-command gates

Clone the independent hardware suite once alongside this checkout:

```sh
gh repo clone lumirth/hachiware ../hachiware
```

```sh
python3 tools/check.py --out out/check-1
python3 tools/verify_retail.py --out out/retail-1
```

Each output directory must be new. `check.py` runs formatting, default and trace-feature
tests, Clippy, a release build, host-tool unit tests, independent fixture generation and
fixture execution. Pass `--hachiware PATH` if the suite is elsewhere. Rust builds and
local tests do not require the suite checkout. `verify_retail.py` separately requires
private images and runs full event/state partition comparison, snapshot replay, and the
real boot/menu/walking/idle workloads. `--quick` omits walking and idle.

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
./target/release/hachistep run --firmware local-inputs/pokewalker.bin \
  --eeprom local-inputs/eeprom.bin --milliseconds 100 --bus-trace \
  --trace boot-bus.txt --trace-limit 10000 --out out-bus
```

A trace build is intentionally more expensive. Rebuild without the feature for
performance measurements. The CLI reports truncation; a capped trace is not a complete
hardware history. Do not run long private event-vector tests with bus tracing enabled
unless the resulting memory use is intentional.

## Source archives

`tools/package.py --out FILE.zip` includes tracked source and `.git`, but not `target/`,
`out/` or compiler archives. `--private` additionally includes the two supplied images
and `private-observations/`. It requires a clean committed tree.
`DELIVERY-MANIFEST.json` identifies the packaged commit and SHA-256 of every archived
file. Validate archive contents against that manifest.
