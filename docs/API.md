# Embedding the core

The runnable example is `crates/hs-core/examples/replay.rs`:

```sh
cargo run -p hs-core --release --example replay -- FIRMWARE EEPROM
```

The crate documentation also includes a compiled doctest using an original
synthetic branch loop, so it needs no proprietary inputs.

## Construction

`Images` contains borrowed firmware/EEPROM bytes and the EEPROM's persistent
status byte. Construction validates sizes (49,152 and 65,536), rejects invalid
nonpersistent status bits, and copies the bytes into owned machine storage.
Images are not whitelisted by hash. A strange reset vector is encountered by
execution rather than accepted because it resembles retail firmware.

`Machine::new(images)` selects default `Conditions`.
`with_conditions(images, conditions)` selects main/watch/on-chip frequencies,
supply voltage and the explicit uncalibrated ADC reference witness.
`with_persistent_state(images, conditions, Some(sensor_bytes))` additionally loads
the 19-byte BMA nonvolatile image. `None` selects the canonical sensor image.

These parameters describe one model instance, not fast/accurate choices. Source
and physical parameter limitations are documented in STATUS.

## Advance and input consumption

```rust,ignore
let end = machine.now().checked_add(Duration::from_millis(100)).ok_or("overflow")?;
let result = machine.run_until(end, &input_timeline[cursor..], &mut output)?;
cursor += result.inputs_consumed;
```

The horizon is exclusive. Do not remove an input at exactly `end`; the next run
must still see it. Timelines are monotonic and properties cannot be assigned
twice at the same timestamp. Independent same-time changes are applied in one
batch. Reversed horizons and past inputs fail. The host must not run ahead of the
input history it actually knows; no retroactive input insertion is supported.

A custom `Output` implements `fn event(&mut self, event: Event)`. It must not
re-enter the machine. The callback is synchronous and cannot return an I/O error;
a host sink may latch its error and stop issuing further run calls. The CLI
implements this pattern. A no-op sink is `&mut ()`.

## Observation

`registers`, `instruction_pc`, `phase_name`, `statistics`, `retired` and
`interrupt_entries` provide diagnostic state. `peek(address)` is a diagnostic
projection with no guest read side effects; it may clone MCU state and is not a
hot-path primitive.

`display(&mut [u8; 6144])` returns row-major 96x64 shade codes 0..3. The view is
logical controller output, not a calibrated physical panel or frame-clock
simulation. `display_enabled` and `display_start_line` expose useful controller
state. Power-off display rendering is blank.

`firmware`, `ram`, `eeprom`, `eeprom_status`, `sensor_nonvolatile` and `lcd_ram`
return read-only data. No mutable bypass into a guest register or memory array is
part of the ordinary facade. Guest modifications go through normal execution.

## Checkpoint

`snapshot()` creates an owned, typed causal snapshot. `restore(&snapshot)`
restores the same object; `Machine::from_snapshot(&snapshot)` creates another
instance. Snapshot equality and complete subsequent event equality are tested.
This is not serialization, save-state compatibility between versions, or
multi-device network coordination.

A host must also retain its input cursor and output-delivery position. The core
snapshot does not own a host queue. The simplest policy is to drain output,
snapshot the machine, and record the corresponding input position together.

## Reset and power

`Input::ResetPin(false)` asserts active-low MCU reset; `true` releases it.
External component lifetimes are not erased just because the MCU resets.
`power_off(output)` and `power_on(output)` are whole-product lifecycle calls at
the current boundary. They can return a model error; notably, power removal
while external nonvolatile programming is active is unsupported and rejected.

A supply-voltage input is currently an analog-condition change, not an implicit
power/reset call. Do not use a zero supply value as a substitute for `power_off`.

## Concurrency and allocation

Each run holds exclusive mutable access to its machine. Independent instances
can run on separate host threads. There are no global hardware variables or
internal locks. A caller may allocate an output vector; the core itself does not
allocate during ordinary execution. Snapshot/constructor/peek costs should not
be confused with hot execution cost.
