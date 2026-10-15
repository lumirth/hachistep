# Embedding HachiStep

`hs_core::Machine` runs one device. The host supplies images and timestamped inputs,
advances to a deadline, and consumes output events. Use the
[replay example](../crates/hs-core/examples/replay.rs) as a complete caller:

```sh
cargo run -p hs-core --release --example replay -- firmware.bin eeprom.bin
```

## Construct and run

`Machine::new(Images { firmware, eeprom, eeprom_status, sensor_nonvolatile })` copies
borrowed image bytes into the machine. Firmware is 49,152 bytes; EEPROM is 65,536 bytes.
`eeprom_status` contains persistent M95512 bits. `sensor_nonvolatile` is an optional
19-byte sensor image. See [input formats](INPUTS.md).

`Machine::with_conditions` also accepts clock frequencies, supply, temperature, and
battery-sense parameters. Time zero starts powered with oscillators ready at reset-vector
entry. Start with zero supply and deliver a rail rise to exercise cold startup.

```rust,ignore
let end = machine.now().checked_add(Duration::from_millis(100)).ok_or("overflow")?;
let remaining = &timeline[cursor..];
let count = remaining.partition_point(|change| change.at < end);
let result = machine.run_until(end, &remaining[..count], &mut output)?;
cursor += result.inputs_consumed;
```

The horizon is exclusive. Retain an input at exactly `end` for the next call. Order
inputs by timestamp, assign each property once per timestamp, and keep same-time
batches together. The core validates the whole slice before running, including records
beyond the horizon. Passing just the current interval avoids repeated validation.
Past inputs and reversed horizons fail before execution.

`Output::event` receives timestamped `Event` values and returns `ControlFlow<()>`.
A closure with that signature implements `Output`; `Vec<Event>` collects events and
`()` discards them. The callback is synchronous and cannot re-enter the machine.

Return `Break(())` to stop after all effects at the current timestamp. The callback
still receives remaining events at that timestamp, and that input batch is consumed.
`RunResult.reason` is `StopReason::Output`; `now` is the exclusive horizon one 64.64
quantum after the completed instant. Resume with the unconsumed input suffix.
`StopReason::Horizon` means the requested deadline was reached without a stop request.

A core fault returns `Err` and stops the session. Inspection and capture remain
available. Restore a healthy checkpoint or construct another machine to continue.
Input validation errors and failed restores leave a healthy machine usable.

## Display and audio

`display(&mut [u8; 6144])` writes row-major 96 by 64 pixels. Values are programmed
PWM drive averaged over FRC frames, scaled to 0..255 and rounded. The image uses
current RAM and geometry. An inactive display or scan clock produces zero pixels.
Map these drive values to your panel tint and contrast response.

`display_contrast()` returns electronic volume from 0 to 63. `display_enabled()` and
`display_start_line()` expose controller state. `display_drive()` returns the current
digital COM/SEG output, including modulation and polarity, without advancing time.

`machine.audio(sample_rate)` creates an `Audio` renderer at the current time and
buzzer drive. Feed each output event to `audio.event(event, samples)`, then call
`audio.advance(result.now, samples)` after each run. Both accept a callback over
borrowed mono `i16` blocks. Advance through silence as well.

Rates range from 1000 to 192000 Hz. The renderer uses band-limited synthesis with DC
removal and about eight samples of filter delay. The host owns playback, gain, and
acoustic filtering. After restoration, discard queued audio and create a new renderer.

## Persistence and editing

`Event::NvByte` carries each persistent cell update. `NvCommit` or `NvInterrupted`
closes its address range. Wrapped EEPROM writes report the whole page range, with byte
events identifying the affected cells. Flash commits describe individual programming
pulses; the guest owns its complete program-and-verify procedure.

`firmware()`, `ram()`, `eeprom()`, `eeprom_status()`, `sensor_nonvolatile()`,
`lcd_ram()`, and `lcd_icons()` expose current bytes. `firmware()` returns an owned
image, including partial programming progress. Inspection does not finish a write.

`write_ram(address, bytes)` edits guest addresses `0xF780..0xFF80`.
`write_eeprom(address, bytes)` edits the 64 KiB EEPROM array and requires
`eeprom_busy()` to be false. Both validate the complete range, do not wrap, and run
between execution calls. They emit no guest events. Fetched instructions and buffered
writes retain their contents, so later guest work can overwrite an edit. The host owns
firmware-specific layouts, checksums, and persistence of its edits.

Use [snapshots](SAVE_STATES.md) for exact continuation. Persistent image exports alone
cannot resume work in progress.

## Connections and physical inputs

`Event::Infrared` reports emission changes. Deliver incident optical levels through
`Input::InfraredLevel` before advancing past their timestamps. The host chooses peers,
channel delay, buffering, and transport. An output stop on emission returns control
before later effects; choose run horizons within the input history you know.

`Input::Power` connects or disconnects the common rail. `power_off(output)` and
`power_on(output)` apply the same operation at the current boundary and complete it
before returning. Supply inputs change the configured voltage. Physical time continues
without power; component readiness, reset, and retention follow the rail history.

`Input::ResetPin(false)` asserts active-low RES; `true` drives it high. Internal
release takes eight reference-clock edges. `Input::NmiPin` defaults high and preserves
its level through MCU reset. Unsupported boot/test straps return
`Error::UnsupportedResetMode` at reset release.

`Input::AnalogPin` and `Input::DigitalPin` supply optional package-pin fixture drives;
`None` releases them. [The CSV reference](INPUTS.md#input-csv) lists names and units.

## Diagnostics and host cost

`registers()`, `instruction_pc()`, `statistics()`, `retired()`, and
`interrupt_entries()` expose diagnostics. `peek(address)` reads without guest side
effects and can be expensive. `phase_name()` is an implementation diagnostic whose
labels can change. Use `sleeping()` and run results for execution control.

Counters accumulate through hardware resets and reset on snapshot restoration. Use
observation differences to measure an interval. The `diagnostic` module exposes
components for hardware experiments; normal callers use `Machine`.

Independent instances can run on separate host threads. Ordinary execution allocates
nothing in the core. Construction, snapshots, encoding, and some inspection operations
allocate. Frontend output buffers have their own allocation policy.
