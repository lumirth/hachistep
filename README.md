# HachiStep

HachiStep is a Rust Pokéwalker emulator core for applications and custom firmware
development. It runs retail and custom images through the H8/38606 CPU and connected
hardware. Applications supply timestamped physical inputs and choose when to advance
the device. The core provides display data, sound and infrared events, persistent
storage, and exact session captures.

Retail integration checks cover boot, menus, walking, Dowsing, Poké Radar, saving
and peer exchanges. The [hardware accuracy guide](docs/ACCURACY.md) explains which
behavior has documented support and where physical measurements are still needed.

## Embed a device

Use Rust 1.95 or newer. From your application's directory, add the core from this
checkout:

```sh
cargo add hs-core --path /path/to/hachistep/crates/hs-core
```

This complete program runs an original two-byte branch loop, so it needs no retail
firmware. It checks that the emulated CPU ran:

```rust
use hs_core::{Images, Machine, Time};

fn main() -> Result<(), hs_core::Error> {
    let mut firmware = vec![0; 49_152];
    firmware[..2].copy_from_slice(&0x0100u16.to_be_bytes());
    firmware[0x100..0x102].copy_from_slice(&[0x40, 0xfe]);
    let mut device = Machine::new(Images {
        firmware: &firmware,
        eeprom: &[0xff; 65_536],
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })?;
    device.run_until(Time::from_micros(100), &[], &mut ())?;
    assert!(device.retired() > 0);
    Ok(())
}
```

The horizon is exclusive and unfinished hardware work survives the return. A frontend
can run to its next presentation deadline, consume output events synchronously, and
request an early return for communication. The application controls host speed and
storage. Ordinary execution allocates nothing with an allocation-free output consumer.

The [embedding guide](docs/API.md) covers inputs, output stops, display and audio,
persistence, state editing, and save states. The
[replay example](crates/hs-core/examples/replay.rs) demonstrates a frontend loop with
button input, 60 Hz presentation, streamed PCM, and restoration:

```sh
cargo run -p hs-core --release --example replay -- FIRMWARE EEPROM
```

The embedding API is being refined before freezing it. Component-level experiments
use the separate `diagnostic` namespace.

## Run firmware from the command line

Supply your own 49,152-byte firmware and 65,536-byte EEPROM images:

```sh
cargo build --workspace --release --locked
mkdir -p out
./target/release/hachistep run \
  --firmware inputs/pokewalker.bin --eeprom inputs/eeprom.bin \
  --milliseconds 10000 --out out/home
```

Use a new output directory. It receives persistent images, memory/controller dumps,
`frame.pgm`, `state.bin`, and `report.json`. The input files remain intact. Resume the
captured session with an absolute emulated endpoint:

```sh
./target/release/hachistep run --load-state out/home/state.bin \
  --milliseconds 12000 --out out/resumed
```

[BUILD](docs/BUILD.md) covers toolchains and native/Wasm commands.
[INPUTS](docs/INPUTS.md) describes physical input CSVs and image requirements.
[SAVE_STATES](docs/SAVE_STATES.md) explains exact captures and persistent exports.
The source is MIT licensed. Private firmware and derived artwork retain their own terms.

## Develop and verify

The [design contract](docs/DESIGN.md) explains execution, component ownership and
performance requirements. [CONTEXT](CONTEXT.md) defines the project vocabulary, and the
[source catalogue](docs/SOURCES.md) links the hardware evidence.

Independent guest diagnostics live in [hachiware](https://github.com/lumirth/hachiware).
Clone it beside this repository and run the standard checks:

```sh
gh repo clone lumirth/hachiware ../hachiware
uv run tools/check.py --out out/check-1
uv run tools/mutation_check.py --out out/mutations-1
uv run tools/verify_retail.py --out out/retail-1
```

The standard checks need no private firmware. Retail verification needs the private
images and checks software regressions, complete event histories and restoration.
[TESTING](docs/TESTING.md) explains what each result establishes and how to compare
performance after verifying behavior.
