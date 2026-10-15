# HachiStep

HachiStep is a Pokéwalker emulator written in Rust, built for hardware accuracy and
fast execution of retail and custom firmware. It provides an embeddable core and a
headless CLI for firmware development, scripted sessions, and benchmarking.

The project aims to make custom firmware development practical before flashing a
physical device. Hardware fidelity and speed share one implementation. The
[design goals](docs/DESIGN.md) describe that commitment.

## Run firmware

Install Rust 1.95 or newer, then build:

```sh
cargo build -p hs-cli --release --locked
```

Supply a raw 48 KiB firmware image and a 64 KiB EEPROM image:

```sh
./target/release/hachistep run --firmware firmware.bin --eeprom eeprom.bin \
  --milliseconds 10000 --out out/session
```

Open `out/session/frame.pgm` to see the final display. The directory also contains
persistent images, a resumable `state.bin`, and `report.json` with execution results.
Use a new output directory for each run. Firmware and EEPROM images are supplied
separately from this repository.

Use `--input workloads/menu.csv` for a scripted session. See
[CLI inputs and outputs](docs/INPUTS.md) for the timeline format and
[save states](docs/SAVE_STATES.md) to resume a session.

## Embed the core

`hs_core::Machine` accepts firmware, EEPROM, and timestamped physical inputs. The host
chooses how far to run and receives timestamped output events. It can render display
pixels and stream PCM audio through the same API.

The [embedding guide](docs/API.md) covers the calling contract. Run the complete
[frontend example](crates/hs-core/examples/replay.rs) with your images:

```sh
cargo run -p hs-core --release --example replay -- firmware.bin eeprom.bin
```

## Develop and compare

[Build instructions](docs/BUILD.md) cover development tools, tracing, and WebAssembly.
[Behavioral validation](docs/TESTING.md) runs independent hardware diagnostics and
complete firmware sessions. [Hachiware](https://github.com/lumirth/hachiware) compares
conformance and per-workload speed against other emulators, including PocketWalker.

The [hardware notes](docs/HARDWARE.md) collect source conflicts, assumptions, and
remaining fidelity work relevant to firmware authors.

[MIT license](LICENSE).
