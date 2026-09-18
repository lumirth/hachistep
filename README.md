# HachiStep

A Rust Pokéwalker emulator core for retail and custom firmware, built for use in
downstream applications. The goal is faithful hardware behavior and high performance
by default through a compact, coherent architecture. The core runs
unmodified retail firmware through boot, menus, motion processing and idle. The
[design](docs/DESIGN.md) defines the intended behavior;
[hardware references](docs/SOURCES.md) explain the evidence and model choices.

## Build and run

Use Rust 1.95+ for the core. The supporting tools use Python's standard library;
[uv](https://docs.astral.sh/uv/getting-started/installation/) selects the interpreter
from `.python-version`. Fetch the locked crates once, then builds can run offline:

```sh
cargo fetch --locked
cargo build --workspace --release --locked --offline
cargo test --workspace --locked --offline

mkdir -p out
./target/release/hachistep run \
  --firmware inputs/pokewalker.bin --eeprom inputs/eeprom.bin \
  --milliseconds 10000 --out out/home
```

Supply your own 49,152-byte firmware and 65,536-byte EEPROM images. Create ignored
`inputs/` when placing them there, or pass paths elsewhere. Put generated reports,
traces and recordings under ignored `out/`. The MIT license covers the source;
private firmware and derived artwork or recordings retain their own terms.
See [INPUTS](docs/INPUTS.md) and [SOURCES](docs/SOURCES.md).

`out/home` must be new. It receives persistent images, memory/controller dumps,
`frame.pgm`, `state.bin` and `report.json`. The CLI creates new files and leaves input
files intact. Guest writes still affect the emulated nonvolatile cells.

## Resume an exact session

```sh
./target/release/hachistep run --load-state out/home/state.bin \
  --milliseconds 12000 --out out/resumed
```

The endpoint and any CSV inputs use absolute emulated time. An EEPROM export starts a
fresh session; a native save state retains unfinished CPU accesses, serial shifts, clock
phase, device history and programming operations. Loading validates a complete candidate
before replacing a session. The native format remains changeable before release, without
versions or compatibility layers. See [SAVE_STATES](docs/SAVE_STATES.md).

## Implementation

`hs-core` owns one resumable H8 interpreter, physical bus accesses, rational clocks,
interrupt admission, GPIO, timers, RTC, watchdog, ADC, comparators, AEC, SSU, SCI/IrDA,
IIC2 and internal flash programming. External EEPROM, accelerometer and LCD controllers
consume the resolved board signals. RAM and flash execute through the same interpreter.

Time advances to the next consequence and respects exclusive run horizons. Ordinary
execution allocates nothing with an allocation-free output sink. Construction,
inspection and explicit save/load may allocate. Production source forbids unsafe Rust.
Borsh encodes selected hardware state; SHA-256 identifies firmware and checks state
files. Clap owns CLI argument validation and help.

`hs-cli` accepts physical input CSVs, exports persistent images, produces run reports
and captures bounded traces. The current audio renderer converts captured buzzer events
to WAV through `tools/render_audio.py`. Frontends own device playback and file/slot
management. [API](docs/API.md) describes the available embedding interface;
[DESIGN §12](docs/DESIGN.md#12-outputs-and-presentation) defines the intended output support.

## Verification

The independent diagnostic suite lives in
[hachiware](https://github.com/lumirth/hachiware):

```sh
gh repo clone lumirth/hachiware ../hachiware
uv run tools/check.py --out out/check-1
uv run tools/verify_retail.py --out out/retail-1
```

Each destination must be new. Standard checks include debug/release/trace Rust tests,
formatting, Clippy, host tools and independent guest diagnostics. Retail verification
additionally needs private images and checks boot, menus, walking, idle, persistent saves,
power interruption and infrared timeout, including event histories and restoration.
Expected hardware behavior
comes from documented independent cases; retail hashes are software regression evidence.
[TESTING](docs/TESTING.md) explains the coverage and [BUILD](docs/BUILD.md) lists
toolchain requirements and commands.

## Development

Read [DESIGN](docs/DESIGN.md), [CONTEXT](CONTEXT.md), and the relevant
[hardware references](docs/SOURCES.md) before changing behavior. Decisions come
from hardware documentation, observations, the matching `pw` decompilation, and
inferences that explain the mechanism for arbitrary firmware. Development history and
work in progress belong in commits, issues, and the task discussion.
