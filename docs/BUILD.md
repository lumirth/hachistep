# Building HachiStep

Use Rust 1.95 or newer with a native linker. Python tools require Python 3.10 or newer
and use the standard library. `uv run` selects the version in `.python-version`.

```sh
cargo build --workspace --release --locked
```

The CLI is `target/release/hachistep`, or `hachistep.exe` on Windows. Add `--offline`
after fetching the locked dependencies to build without network access.

## Development

Install rustfmt and Clippy, then check the workspace:

```sh
rustup component add rustfmt clippy
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

See [behavioral validation](TESTING.md) to run Hachiware diagnostics and firmware sessions.
Build with `--profile profiling` for symbols, then use `tools/profile.py` with your
firmware and EEPROM to capture a macOS stack sample. `--input` selects a scripted
session; `--out` selects a new directory for the sample and invocation details.

## Bus tracing

```sh
cargo build -p hs-cli --release --locked --features trace
./target/release/hachistep run --firmware firmware.bin --eeprom eeprom.bin \
  --milliseconds 100 --bus-trace --trace out/bus.txt --trace-limit 10000
```

The `trace` feature adds bus observations. Rebuild without it for speed measurements.
`trace_complete` in the report indicates whether the trace fit its record limit.

## WebAssembly

Install [Wasmtime](https://docs.wasmtime.dev/cli.html) and the WASI Rust target:

```sh
rustup target add wasm32-wasip1
cargo build -p hs-cli --release --locked --target wasm32-wasip1
```

The Hachiware adapter and comparison tools also accept the `.wasm` CLI. They invoke
Wasmtime with access to the input and output directories. Browser applications need
host bindings for the core API.
