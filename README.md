# HachiStep

A Rust Pokéwalker emulator core for retail and custom firmware. The goal is one
highly accurate, maximally fast execution path with a compact implementation.
The core currently runs unmodified retail firmware through boot, menus, motion
processing and idle. [STATUS](docs/STATUS.md) records the implemented hardware
and the remaining fidelity work.

## Build and run

Use Rust 1.95+ and Python 3.10+ for the supporting tools. Fetch the locked crates
once, then builds and checks can run offline:

```sh
cargo fetch --locked
cargo build --workspace --release --locked --offline
cargo test --workspace --locked --offline

./target/release/hachistep run \
  --firmware local-inputs/pokewalker.bin --eeprom local-inputs/eeprom.bin \
  --milliseconds 10000 --out out-home
```

Supply your own 49,152-byte firmware and 65,536-byte EEPROM images. Private
inputs stay in ignored `local-inputs/`; the MIT license covers the source, not
firmware, artwork or recordings. See [SOURCES](docs/SOURCES.md).

`out-home` must be new. It receives persistent images, memory/controller dumps,
`frame.pgm`, `state.bin` and `report.json`. The CLI creates new files and leaves
input files intact. Guest writes still affect the emulated nonvolatile cells.

## Resume an exact session

```sh
./target/release/hachistep run --load-state out-home/state.bin \
  --milliseconds 12000 --out out-resumed
```

The endpoint and any CSV inputs use absolute emulated time. An EEPROM export
starts a fresh session; a native save state retains unfinished CPU accesses,
serial shifts, clock phase, device history and programming operations. Loading
validates a complete candidate before replacing a session. The native format
remains changeable before release, without versions or compatibility layers.
See [SAVE_STATES](docs/SAVE_STATES.md).

## Implementation

`hs-core` owns one resumable H8 interpreter, physical bus accesses, rational
clocks, interrupt admission, GPIO, timers, RTC, watchdog, ADC, comparators, AEC,
SSU, SCI/IrDA, IIC2 and internal flash programming. External EEPROM, accelerometer
and LCD controllers consume the resolved board signals. RAM and flash execute
through the same interpreter.

Time advances to the next consequence and respects exclusive run horizons.
Ordinary execution allocates nothing with an allocation-free output sink.
Construction, inspection and explicit save/load may allocate. Production source
forbids unsafe Rust. Borsh encodes selected hardware state; SHA-256 identifies
firmware and checks state files. Clap owns CLI argument validation and help.

`hs-cli` accepts physical input CSVs, exports persistent images, produces run
reports and captures bounded traces. Frontends, audio rendering and file/slot
management remain outside the core. [API](docs/API.md) describes embedding.

## Verification

The independent diagnostic suite lives in
[hachiware](https://github.com/lumirth/hachiware):

```sh
gh repo clone lumirth/hachiware ../hachiware
python3 tools/check.py --out out/check-1
python3 tools/verify_retail.py --out out/retail-1
```

Each destination must be new. Standard checks include debug/release/trace Rust
tests, formatting, Clippy, host tools and independent guest diagnostics. Retail
verification additionally needs private images and checks reviewed boot, menu,
walking and idle observations, event histories, partitioning and restoration.
Expected hardware behavior comes from documented independent cases; retail
hashes are software regression evidence. [TESTING](docs/TESTING.md) explains the
coverage and [BUILD](docs/BUILD.md) records tested toolchains and dependencies.

## Development

Read [DESIGN](docs/DESIGN.md), [CONTEXT](CONTEXT.md), the relevant
[architecture](docs/ARCHITECTURE.md) section and owner research before changing
behavior. [HANDOFF](docs/HANDOFF.md) tracks concrete remaining work. Decisions
come from hardware documentation, observations, the matching `pw` decompilation
and explicit inferences that explain the mechanism for arbitrary firmware.
