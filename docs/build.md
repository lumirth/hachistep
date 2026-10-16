# Build hachistep

Rust 1.95 or newer and a native linker are enough to build the core and CLI:

```sh
git clone https://github.com/lumirth/hachistep.git
cd hachistep
cargo build --workspace --release --locked
```

The build produces `target/release/hachistep`, or `hachistep.exe` on Windows.
The [CLI guide](cli.md) starts a firmware session. For an application using the
library, the [embedding guide](api.md) shows how to depend on `crates/hs-core`
and run a device.

The Python tools require Python 3.10 or newer and use the standard library.
`uv run` selects the version in `.python-version`. After fetching locked Cargo
dependencies, add `--offline` to build without network access.

## Build for WebAssembly

Install the WASI Rust target:

```sh
rustup target add wasm32-wasip1
cargo build -p hs-cli --release --locked --target wasm32-wasip1
```

The executable is `target/wasm32-wasip1/release/hachistep.wasm`.
Install [Wasmtime](https://docs.wasmtime.dev/cli.html) to run it. The hachiware adapter
accepts that path as its executable and grants Wasmtime access to the required files.

This WASI build runs the CLI's file-based workflow under Wasmtime. It also lets
hachiware compare native and WebAssembly runs of the same session. A browser frontend
uses `hs-core` with its own JavaScript bindings instead.

## Trace bus accesses

```sh
cargo build -p hs-cli --release --locked --features trace
mkdir -p out
./target/release/hachistep run --firmware firmware.bin --eeprom eeprom.bin \
  --milliseconds 100 --bus-trace --trace out/bus.txt --trace-limit 10000
```

`out/bus.txt` contains the timestamped accesses. Check `trace_complete` in the report
to see whether the capture fit its limit. Use a build without `trace` when measuring
execution speed.

## Profile a session

On macOS, the profiling build retains symbols for the stack sampler:

```sh
cargo build -p hs-cli --profile profiling --locked
python3 tools/profile.py --runner target/profiling/hachistep \
  --firmware firmware.bin --eeprom eeprom.bin --out out/profile
```

The sample is in `out/profile/sample.txt`. Add `--input inputs.csv` to investigate a
particular session, and increase `--milliseconds` if it ends before sampling does.
Choose a new output directory for each profile.

## Check a change

For core changes, read the [guiding principles](design.md) and relevant
[hardware notes](hardware.md).

```sh
rustup component add rustfmt clippy
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

`cargo doc -p hs-core --no-deps --open` opens the generated method reference.
Use [hachiware](testing.md) to check the hardware behavior affected by a change,
then the relevant firmware sessions. Completed checks remain useful until a change
or a failure gives us a reason to repeat them.
