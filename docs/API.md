# Embedding the core

The runnable example is `crates/hs-core/examples/replay.rs`:

```sh
cargo run -p hs-core --release --example replay -- FIRMWARE EEPROM
```

The crate documentation also includes a compiled doctest using an original synthetic
branch loop, so it needs no proprietary inputs.

## Construction

`Images` contains borrowed firmware/EEPROM bytes and the EEPROM's persistent status
byte. Construction validates sizes (49,152 and 65,536), rejects invalid nonpersistent
status bits, and copies the bytes into owned machine storage. Firmware identity does
not restrict execution. The CPU reads the supplied reset vector through the normal bus.

`Machine::new(images)` selects default `Conditions`. `with_conditions(images,
conditions)` selects main/watch/on-chip frequencies, supply voltage, temperature and the
nominal battery-sense voltage drop. AVCC follows supply by default;
`avcc_override_millivolts` supplies an optional external analog-supply fixture.
`battery_sense_drop_millivolts` defaults to 600; this inferred effective circuit
parameter is independent of firmware and EEPROM calibration. P84 must actually drive
high to enable the sense path. `with_persistent_state(images, conditions,
Some(sensor_bytes))` additionally loads the 19-byte BMA nonvolatile image. `None`
selects the default sensor image.

Time zero is a powered, oscillator-ready board at reset-vector entry. This explicit
initial state avoids imposing an arbitrary battery-insertion history. Set the initial
supply to zero and supply a timestamped rail rise to exercise a discharged RES capacitor
and cold oscillator startup through the same model. Cold RAM has the deterministic zero
realization. Source and physical parameter choices are documented in the [hardware
references](SOURCES.md).

## Advance and input consumption

```rust,ignore
let end = machine.now().checked_add(Duration::from_millis(100)).ok_or("overflow")?;
let remaining = &input_timeline[cursor..];
let count = remaining.partition_point(|change| change.at < end);
let result = machine.run_until(end, &remaining[..count], &mut output)?;
cursor += result.inputs_consumed;
```

The horizon is exclusive. Do not remove an input at exactly `end`; the next run must
still see it. Timelines are monotonic and properties cannot be assigned twice at the
same timestamp. Independent same-time changes are applied in one batch. Reversed
horizons and past inputs fail. The host must not run ahead of the input history it
actually knows; no retroactive input insertion is supported.

For frequent calls, pass the portion of the ordered timeline before that call's horizon,
as above. Input validation then visits each consumed event once, avoiding repeated scans
of future input.

A custom `Output` implements `fn event(&mut self, event: Event) -> ControlFlow<()>`,
using `std::ops::ControlFlow`. Return `Continue(())` to keep running or `Break(())` to
return control after all effects at the current timestamp finish. Further events at
that timestamp are still delivered, and the entire input batch is consumed. The callback
is synchronous and must not re-enter the machine. A host sink can retain an I/O error
and return `Break(())`; the CLI uses this pattern. A no-op sink is `&mut ()`.

After a stop request, `RunResult.now` is the exclusive horizon one 64.64 time quantum
(`2^-64` seconds) after the completed instant. It can precede the requested `end`. New
input may start at the returned horizon; the completed instant is already past. Resume
through another `run_until` call with the unconsumed inputs. The stop request belongs to
the current call and is absent from save states. Immediate power operations always
finish their complete operation and return.

Persistent updates carry their data in `NvByte`; `NvCommit` or `NvInterrupted` then
closes an address range containing the affected bytes. A wrapped EEPROM write spans
the whole page; its byte events identify the selected cells. Internal flash reports
settled cell changes during a pulse
and closes that pulse when software ends it or hardware interrupts it. A pulse commit
does not assert that the guest's complete program/erase-and-verify algorithm succeeded.
Inspection and capture emit no events; ordinary reads retain the direct array path.

A core fault stops that session, including immediate power operations. It may have
occurred after part of a hardware effect, so reconnecting cannot safely resume it.
Observation and capture remain available; explicitly restore a healthy checkpoint or
construct a new machine to recover. Input validation and failed restore are checked
before mutation and do not poison a healthy machine.

## Execution speed and connections

The caller controls execution speed by pacing run calls against its host clock. Hardware
timing stays in emulated time. The core owns one device; peer selection, transport and
buffering belong to the application.

`Event::Infrared` reports timestamped emission changes. `Input::InfraredLevel` supplies
incident optical levels through the timestamped input timeline. Deliver inputs before
advancing past their timestamps. Returning `Break(())` from the output callback on an
emission change gives the caller control before any later effects. The caller still
supplies a horizon within its known input timeline. The connection design and timing
responsibilities are in [DESIGN
§13.5](DESIGN.md#135-execution-pacing-and-external-connections).

## Observation

`registers`, `instruction_pc`, `phase_name`, `statistics`, `retired` and
`interrupt_entries` provide diagnostic state. `peek(address)` is a diagnostic projection
with no guest read side effects. It may clone MCU state, so frequent inspection can be
expensive.

`display(&mut [u8; 6144])` returns row-major 96x64 shade codes 0..3 derived from
controller RAM and settings. Frontends supply the panel's visual appearance.
`display_enabled` and `display_start_line` expose useful controller state. Power-off
display rendering is blank.

`display_drive()` projects the LCD controller's digital output at the current
observation point: selected COM (128 for the icon), two 64-bit SEG masks, and AC
polarity. It includes PWM/FRC, the output latch, and frame-latched start line. It does
not advance the guest or alter snapshot state. Power-save and display-off return
inactive drive. This is separate from analog glass response.

Audio output currently consists of timestamped `Event::Buzzer` drive changes. The
offline `tools/render_audio.py` tool converts a captured trace into WAV. The Rust API
does not yet provide the reusable PCM conversion required by
[DESIGN §12.2](DESIGN.md#122-buzzer-output).

`firmware`, `ram`, `eeprom`, `eeprom_status`, `sensor_nonvolatile`, `lcd_ram` and
`lcd_icons` return read-only data. RAM and EEPROM edits use the operations below.

`firmware()` returns an owned 48-KiB image of the current flash cells, including the
physical progress of an unfinished pulse. It may differ from the firmware loaded at
construction. The CLI exports this as `flash.bin`; the input file is never modified.
Exporting or peeking at flash does not perform a guest read, trigger protection or
finish a pulse. A raw image preserves readable bytes; an exact snapshot additionally
preserves intermediate cell charge and controls.

## State editing

`write_ram(address, bytes)` copies bytes into RAM at guest addresses `0xF780..0xFF80`.
`write_eeprom(address, bytes)` copies bytes into the EEPROM's 64 KiB array. Both require
exclusive mutable access between run calls. They validate the whole range before
editing; ranges do not wrap. Neither advances time, performs a guest bus access, emits
events or clears a core fault. A faulted session rejects edits until restored.

Fetched instructions, pending CPU accesses and serial buffers retain their contents.
Future reads see edited memory; a pending guest write can overwrite it. EEPROM edits
require `eeprom_busy()` to be false so they cannot change the starting cells of an
ongoing programming cycle. Buffered serial writes can still commit afterward. Direct
edits bypass guest write protection, and the frontend owns persistence of its edits.

The caller supplies any firmware layout, checksums and related value updates. For
example, it may read a counter through `ram()`, calculate a new value and write its
encoded bytes through `write_ram`. Multiple edits between run calls take effect before
execution resumes. [mGBA's raw and bus access APIs][mgba-access] and
[SameBoy's direct memory access][sameboy-access] provide precedents for keeping these
operations distinct from physical input delivery. CPU and peripheral register mutation
is not currently exposed by `Machine`.

[mgba-access]: https://github.com/mgba-emu/mgba/blob/master/include/mgba/core/core.h
[sameboy-access]: https://github.com/LIJI32/SameBoy/blob/master/Core/gb.h

## Checkpoint

`snapshot()` captures typed state without advancing the machine. Its `encode()` method
produces a native file; `Snapshot::decode(bytes)` validates a complete candidate.
`restore(&snapshot)` returns an error on a different original firmware identity and
leaves the live machine intact. `Machine::from_snapshot` constructs another instance
from the full captured state. Diagnostic counters reset on both restoration paths. See
[save states](SAVE_STATES.md) for the hardware contract, bounded encoding, CLI usage and
exclusions. There is no version or compatibility machinery before release.

A host must also retain its input cursor and output-delivery position. The core snapshot
does not own a host queue. The simplest policy is to drain output, snapshot the machine,
and record the corresponding input position together.

## Reset and power

`Input::ResetPin(false)` asserts active-low MCU reset; `true` raises the pin. The
internal reset releases after eight actual reference-clock edges. A new low level
restarts qualification. WDT's separate 512-ROSC hold remains independent. This fixture
drives the actual package pin, overriding its ordinary RC voltage. With no fixture, the
board's RES capacitor retains charge across rail segments and determines whether
restoration causes a reset. External component lifetimes are not erased just because the
MCU resets.

`power_off(output)` disconnects the common rail; `power_on(output)` reconnects the
configured voltage at the current boundary. Their timestamped equivalents are
`Input::Power`. A supply input changes the configured voltage without reconnecting an
explicitly disconnected battery. `powered()` means that the effective rail is nonzero,
not that every chip or clock is ready.

Physical time continues while the rail is absent. Short interruptions retain RAM, RTC
state and unfinished CPU work; sustained undervoltage exhausts the selected
volatile-cell retention budget. MCU, EEPROM, sensor and LCD availability follow their
own supply domains. Oscillator startup prevents cold execution before main-clock
readiness; the watch crystal can become ready later without holding the main CPU.
Interrupted nonvolatile programming preserves partial cells through the same model used
during normal progress.

`Input::NmiPin(bool)` controls the dedicated NMI input, separate from IRQ enables and
flags. It defaults high and preserves its physical level across MCU reset. NMI low with
TEST low at external reset release admits the manufacturer boot service; TEST high
selects the quiescent test state. See the boot protocol in
[INPUTS](INPUTS.md).

`Input::AnalogPin { pin, millivolts }` and `Input::DigitalPin { pin, level }` accept
optional fixture drives (`None` releases them). The actual names/variant field spelling
are defined in `signals.rs`; see INPUTS for the CLI equivalents. These fixtures flow
through the selected pin functions and their owning peripherals.

Supply changes affect the analog network and functional availability. Falling below a
chip's operating range freezes or interrupts its physical work; this does not by itself
assert a clean brownout reset. RES charge and retention are separate mechanisms. Their
nominal constants and worked examples are in
[the power contract](research/power-and-reset.md).

## Concurrency and allocation

Each run holds exclusive mutable access to its machine. Independent instances can run on
separate host threads. There are no global hardware variables or internal locks. A
caller may allocate an output vector; the core itself does not allocate during ordinary
execution. Snapshot/constructor/peek costs should not be confused with hot execution
cost.

Flash construction reserves at most 384 charge-page slots (about 3 MiB of address
space); a slot is populated only when its page is exposed to a pulse. Normal firmware
uses direct byte-array reads. Snapshot cloning preserves that reserve so even later
custom firmware programming does not allocate in a run.
