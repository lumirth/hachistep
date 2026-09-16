# Build and tooling

## Requirements

The source is edition-2021 Rust with a declared `rust-version = "1.74"` and no
external crates. The declared floor was **not** separately tested on Rust 1.74.
The tested compiler is recorded below. Python tools require Python 3.10+ and
only the standard library. A system linker is needed by the native Rust build.

```sh
cargo build --workspace --release --locked --offline
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
```

`--offline` does not install Rust, a linker, Clippy or rustfmt. Install your normal
Rust toolchain independently when those tools are absent. This project contains
no installer, downloader, hidden build-time network request or cloud CI workflow.

## Tested environment

The execution sandbox initially had no Rust installation and no ordinary outbound
download access. An existing public CI artifact supplied a host compiler:

```
rustc 1.97.0-dev (e638c6cfe 2026-07-15)
cargo 1.97.0-dev (c980f4866 2026-06-30)
rustfmt 1.9.0-dev (e638c6cfea 2026-07-15)
Host: x86_64-unknown-linux-gnu
Compiler repository: risc0/rust
Commit: e638c6cfea1eff5fbbb24a27e60538e3760d21b8
```

This is the RISC Zero fork's host Rust toolchain, not a claim about the latest
upstream stable release. The emulator was compiled for ordinary Linux x86-64,
not the RISC Zero guest. The source does not depend on that fork or target.
Neither macOS/ARM64 execution nor the declared minimum compiler was tested here.
The compiler distribution is not in the delivery ZIP.

That artifact names rustdoc `rustdoc_tool_binary` and has rustfmt but no
`cargo-fmt` wrapper. The check scripts detect the rustdoc name and invoke
rustfmt directly. With a conventional toolchain, ordinary `cargo fmt --check`
also works. The archive's recorded tool versions are in `evidence/`.

## Single-command gates

```sh
python3 tools/check.py --out out/check-1
python3 tools/verify_retail.py --out out/retail-1
```

Each output directory must be new. `check.py` runs formatting, default and
trace-feature tests, Clippy, a release build, host-tool unit tests, independent
fixture generation and fixture execution. `verify_retail.py` separately requires
private images and runs full event/state partition comparison, snapshot replay,
and the real boot/menu/walking/idle workloads. `--quick` omits walking and idle.

On Windows the check/verification scripts select `hachistep.exe`; direct shell
examples in the docs use Unix executable spelling. Platform portability is an
implementation goal, not a claim of having tested all hosts.

## Output safety

The CLI always opens outputs with create-new semantics. `--out` creates a new
directory before running. It never writes back to input ROM, EEPROM or sensor
files. On a model fault it reports the failing state and can export that stopped
state to the newly created output directory. Trace I/O failures fail the run
rather than silently certifying a complete trace.

Output files are separate domains, not a transactional save container. Files are
synced, and the run report is written last, but interruption can leave a partial
output directory. Keep original inputs and treat a directory missing its complete
report as an interrupted export. The core does not write files.

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
performance measurements. The CLI reports truncation; a capped trace is not a
complete hardware history. Do not run long private event-vector tests with bus
tracing enabled unless the resulting memory use is intentional.

## Delivery archive

`tools/package.py --out FILE.zip` includes tracked source and `.git`, but not
`target/`, `out/` or compiler archives. `--private` additionally includes the two
supplied images and `private-observations/`. It requires a clean committed tree.
`DELIVERY-MANIFEST.json` identifies the packaged commit and SHA-256 of every
archived file. Git can legitimately refresh `.git/index` after an ordinary status
command; check archived bytes, rather than later mutable Git cache metadata, for
archive-integrity validation. The final evidence includes a clean extraction,
new-target-directory rebuild, tests, fixtures and an actual private-firmware boot.
