# Building HachiStep

Use Rust 1.95 or newer with a native linker. Python tools require Python 3.10 or newer
and use the standard library. `uv run` selects the version in `.python-version`.

```sh
cargo build --workspace --release --locked
```

The CLI is `target/release/hachistep`, or `hachistep.exe` on Windows. Add `--offline`
after fetching the locked dependencies to build without network access.

## Development

Install rustfmt and Clippy, then clone Hachiware alongside this checkout:

```sh
rustup component add rustfmt clippy
gh repo clone lumirth/hachiware ../hachiware
uv run tools/check.py --out out/check-1
```

This builds the release CLI, checks formatting and Clippy, and runs Hachiware's guest
diagnostics. Pass `--hachiware PATH` for another checkout location. See
[behavioral validation](TESTING.md) for firmware sessions and speed comparisons.

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
uv run tools/verify_retail.py --firmware firmware.bin --eeprom eeprom.bin \
  --runner target/wasm32-wasip1/release/hachistep.wasm --quick --out out/wasm-1
```

The Hachiware adapter and comparison tools also accept the `.wasm` CLI. They invoke
Wasmtime with access to the input and output directories. Browser applications need
host bindings for the core API.
