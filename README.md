# HachiStep — runnable Rust starter

A new, dependency-free implementation of a Pokéwalker development core. It runs
real firmware; it is not a mock UI, native replacement of firmware functions, or
repackaged earlier HachiStep implementation.

**This is a working starter, not the completed hardware-indistinguishable core.**
The current model executes the supplied, unmodified 48 KiB firmware through boot,
button-driven menus, motion processing and idle transitions. Several timing
rules are provisional and several peripheral modes stop as unsupported. The
exact boundary is in [STATUS](docs/STATUS.md). Successful retail execution must
not be confused with arbitrary-firmware silicon conformance.

## Run

Use Rust/Cargo and, for the supporting tools, Python 3.10 or newer. There are no
crates.io dependencies and no required Python packages.

```sh
cargo build --workspace --release --locked --offline
cargo test --workspace --locked --offline

cargo run -p hs-cli --release --offline -- run \
  --firmware local-inputs/pokewalker.bin --eeprom local-inputs/eeprom.bin \
  --milliseconds 10000 --out out-home
```

`out-home` must not already exist. It receives `frame.pgm`, persistent images,
RAM/controller dumps, and `report.json`. The CLI never overwrites input images
or an existing output file. Guest EEPROM writes still execute normally; not
writing them back to the original host file does not alter the emulated chip.

The delivered **private ZIP includes the supplied firmware and EEPROM in the
Git-ignored `local-inputs/` directory**. These images, derived LCD artwork, and
captured sound are not licensed under the starter's MIT license. Do not publish
this private archive. The original large `pw-inputs.zip` and compiler distribution
are deliberately not bundled. See [input provenance](docs/SOURCES.md).

## Reproduce the demonstrated behavior

```sh
# Boot, menu, motion and 120-second idle runs; includes exact replay tests.
python3 tools/verify_retail.py

# Menu buttons and a separately captured final screen.
cargo run -p hs-cli --release --offline -- run \
  --firmware local-inputs/pokewalker.bin --eeprom local-inputs/eeprom.bin \
  --input conformance/scenarios/menu.csv --milliseconds 6500 --out out-menu

# Physical acceleration only: the firmware itself decides whether to add steps.
cargo run -p hs-cli --release --offline -- run \
  --firmware local-inputs/pokewalker.bin --eeprom local-inputs/eeprom.bin \
  --input conformance/scenarios/walking.csv --milliseconds 61000 --out out-walk

python3 tools/preview.py out-home/frame.pgm out-menu/frame.pgm out-walk/frame.pgm \
  --out captured-frames.html
```

With the packaged images and default conditions, observed results include:

| Scenario | Observed result |
|---|---|
| 10-second boot | Home screen, 6,205,663 retired instructions, 237 interrupt entries. |
| 6.5-second button replay | Menu navigation; 688 timestamped differential buzzer transitions. |
| 61-second synthetic movement | Firmware displays **107 steps**; 16,779,535 retired instructions. |
| 120-second stationary run | Completes without a model fault; display enters power save. |
| Random run partitioning | Complete product event vectors and full machine state agree. |
| Snapshot replay | Subsequent complete event vectors and full machine state agree. |
| Independent guest fixtures | Seven pass: CPU aliases/flags/calls/RAM execution, serial EEPROM programming, infrared transmit and receive. |

The movement input is a synthetic 2 Hz trajectory, not a hardware capture or a
pedometer-accuracy study. The private frame viewer is
`private-observations/index.html`; `private-observations/menu.wav` is ideal-drive
host rendering of the captured buzzer transitions, not a calibrated recording.
Machine-readable execution reports and build/test logs are under `evidence/`.

## What's implemented

`hs-core` contains one resumable H8 executor, incremental decoding, explicit-width
ALU/CCR operations, partial physical accesses, 64.64 timestamps, rational clocks,
fixed hardware composition, interrupts, clock/power controls, GPIO, Timer B1,
Timer W's supported modes, RTC, watchdog, ADC, SSU, asynchronous SCI/IrDA, M95512,
BMA150 and NT7508 owners. RAM execution uses the same executor as flash.

The kernel accepts exact exclusive horizons and physical input timelines. It
retains in-flight CPU/serial/nonvolatile state in typed snapshots. Ordinary
execution with an allocation-free output sink performs no heap allocations;
construction, snapshots, diagnostic inspection and user-chosen collectors may
allocate. Production core code forbids `unsafe`.

`hs-cli` supplies input parsing, bounded event traces, image inspection, raw
persistence export/import, screenshots, SHA-256 identities, and JSON run reports.
Python tools provide independent fixtures, verification, paired measurements,
private-input extraction, PGM viewing, WAV rendering and ZIP packaging.

## Start contributing

Read [HANDOFF](docs/HANDOFF.md) first, then the relevant owner and its tests. The
highest-priority gaps are CPU fetch/microtiming and interrupt-admission fidelity,
clock-transition and same-time race handling, missing MCU modes, and sensor/
analog characterization—not a second interpreter or a new emulator framework.

```sh
python3 tools/check.py                 # offline Rust + host-tool + fixture gates
cargo run -p hs-core --release --example replay -- \
  local-inputs/pokewalker.bin local-inputs/eeprom.bin
```

The documented toolchain floor is declared in Cargo metadata; only the exact
compiler recorded in [BUILD](docs/BUILD.md) and `evidence/` was exercised here.
No GitHub workflow is installed or triggered. The `.git` directory and actual
incremental commits are included in the ZIP.

## Documentation

| Document | Contents |
|---|---|
| [BUILD](docs/BUILD.md) | Build commands, offline setup, tested compiler, output safety. |
| [ARCHITECTURE](docs/ARCHITECTURE.md) | State authority, execution/timing, integration and error contracts. |
| [API](docs/API.md) | Embedding, input horizons, snapshots, reset and power. |
| [INPUTS](docs/INPUTS.md) | Physical input CSV, units, traces, persistent-image round trips. |
| [STATUS](docs/STATUS.md) | Implemented behavior, provisional witnesses and missing hardware. |
| [HANDOFF](docs/HANDOFF.md) | Ordered implementation tasks, exact files and acceptance tests. |
| [TESTING](docs/TESTING.md) | What the checks prove, what they do not, conformance organization. |
| [SOURCES](docs/SOURCES.md) | Firmware, EEPROM, documentation and toolchain provenance. |
| [conformance/README](conformance/README.md) | Core-independent diagnostic-image contract. |
