# Embed hachistep

With a hachistep checkout beside your application, add this to `Cargo.toml`:

```toml
[dependencies]
hs-core = { path = "../hachistep/crates/hs-core" }
```

## Run firmware and draw the screen

You'll need a raw 49,152-byte firmware image and a 65,536-byte EEPROM image.
Put them beside your application's `Cargo.toml`. This program runs ten seconds
and writes the screen to `frame.pgm`:

```rust
use hs_core::{Images, Machine, Time};
use std::{error::Error, fs, io::Write};

fn main() -> Result<(), Box<dyn Error>> {
    let firmware = fs::read("firmware.bin")?;
    let eeprom = fs::read("eeprom.bin")?;
    let mut machine = Machine::new(Images {
        firmware: &firmware,
        eeprom: &eeprom,
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })?;

    machine.run_until(Time::from_micros(10_000_000), &[], &mut ())?;

    let mut pixels = [0u8; 96 * 64];
    machine.display(&mut pixels);
    let mut frame = fs::File::create("frame.pgm")?;
    frame.write_all(b"P5\n96 64\n255\n")?;
    let image = pixels.map(|drive| 255 - drive);
    frame.write_all(&image)?;
    Ok(())
}
```

Use `cargo run --release`, then open `frame.pgm`. In a frontend, send the pixel
buffer to a texture instead. Higher values mean darker pixels; your renderer
chooses the tint.

## Schedule input

Your update loop decides how much device time to run. `run_until` takes an absolute
endpoint, so after running to ten seconds, running to eleven advances one second.
It runs synchronously; your application handles pacing against the host clock.

Input timestamps use that same device clock. Continuing the example, this presses
the center button at ten seconds and releases it at 10.1:

```rust
use hs_core::{Buttons, Input, TimedInput};

let inputs = [
    TimedInput {
        at: Time::from_micros(10_000_000),
        input: Input::Buttons(Buttons {
            center: true,
            ..Buttons::RELEASED
        }),
    },
    TimedInput {
        at: Time::from_micros(10_100_000),
        input: Input::Buttons(Buttons::RELEASED),
    },
];
let result = machine.run_until(Time::from_micros(11_000_000), &inputs, &mut ())?;
println!("Reached {:?}", result.now);
```

Keep inputs in timestamp order and simultaneous changes together. Each value stays
in effect until changed. The endpoint is exclusive, so input at exactly eleven
seconds belongs to the next call. Advance your input cursor by `inputs_consumed`
before passing the remaining inputs again.

## Save and resume

```rust
use hs_core::Snapshot;

fs::write("session.state", machine.snapshot().encode()?)?;
let saved = Snapshot::decode(&fs::read("session.state")?)?;
machine.restore(&saved)?;
```

Keep the matching core revision with saved sessions. Also retain your input cursor
and discard queued audio when restoring, so the application resumes at the same
point as the device. Recreate the audio renderer after restoration.

For a fresh boot with the current persistent data, export the firmware, EEPROM,
EEPROM status, and sensor calibration instead of restoring a snapshot.

## Debugging

RAM and EEPROM edits happen between execution calls. They leave instructions
already fetched and writes already buffered intact. EEPROM programming must finish
before an edit. Your application handles firmware-specific save layouts and checksums.

## Connect devices

For infrared connections, schedule each emission as input to its peer before that
peer advances past the delivery timestamp. Use exact output stops to regain control
at an emission.
The [CLI's link command](CLI.md#connect-two-devices) provides a working implementation.

Run `cargo doc -p hs-core --no-deps --open` for the API reference, including event
handling, audio rendering, and physical conditions.
