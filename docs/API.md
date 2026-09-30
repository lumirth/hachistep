# Embedding the core

`Machine` runs one Pokéwalker. An application loads images, supplies timestamped
physical inputs and advances the device to an emulated-time deadline. It consumes
events during execution and reads display data afterward. Host pacing, files, audio
playback and connection transport belong to the application.

Import embedding types from `hs_core`. The API is being refined before freezing it;
current callers should update with the core.

`diagnostic` exposes CPU, MCU and external-device components for controlled experiments.
Those interfaces follow the hardware implementation and remain outside the embedding
contract. Ordinary applications need no component construction or synchronization.
The [API research](research/emulator-api.md) explains the consumer needs behind this
boundary.

The [replay example](../crates/hs-core/examples/replay.rs) is a complete caller:

```sh
cargo run -p hs-core --release --example replay -- FIRMWARE EEPROM
```

The example requests 300 display frames at a 60 Hz cadence, delivers button inputs,
streams PCM, and verifies save state restoration. A frontend supplies its own window,
audio device and host pacing.

The [README](../README.md#embed-a-device) shows construction with an original synthetic
branch loop. The same example is a compiled crate doctest and needs no private images.

## Construction

`Images` groups the persistent data used to start a new session:

| Field | Contents |
| --- | --- |
| `firmware` | Raw 49,152-byte internal flash, including reset and exception vectors. |
| `eeprom` | Raw 65,536-byte external EEPROM array. |
| `eeprom_status` | Persistent M95512 status bits. WIP and WEL are transient and rejected. |
| `sensor_nonvolatile` | Optional 19-byte BMA150 configuration/calibration image. `None` selects the modeled calibrated default. |

Construction validates the image lengths and status bits, then copies the borrowed
bytes into owned machine storage. The caller can release its buffers afterward.
Firmware identity does not restrict execution. The CPU reads the supplied reset vector
through the normal bus.

`Machine::new(images)` selects default `Conditions`. `with_conditions(images,
conditions)` selects main/watch/on-chip frequencies, supply voltage, temperature and the
nominal battery-sense voltage drop. AVCC follows supply by default;
`avcc_override_millivolts` supplies an optional external analog-supply fixture.
`battery_sense_drop_millivolts` defaults to 600; this inferred effective circuit
parameter is independent of firmware and EEPROM calibration. P84 must actually drive
high to enable the sense path. A supplied sensor image loads its volatile working
registers as at a cold start. These images preserve stored bytes, not an in-progress
session. Use a snapshot to resume partially completed hardware work.

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
still see it. Timelines are ordered by timestamp. A property cannot be assigned twice
at the same timestamp. Independent same-time changes are applied in one batch. Reversed
horizons and past inputs fail. The host must not run ahead of the input history it
actually knows; no retroactive input insertion is supported.

Each call validates the entire supplied slice before execution, including inputs beyond
its horizon. For frequent calls, pass only the portion before that call's horizon, as
above. This bounds validation to that interval. An early output stop leaves an
unconsumed suffix, which the next call validates again. Keep the complete same-time
batch together when partitioning a timeline.

A closure implements `Output` when it accepts `Event` and returns
`std::ops::ControlFlow<()>`. A frontend can borrow its own buffers without creating a
separate callback object:

```rust,ignore
use std::ops::ControlFlow;

let mut events = Vec::new();
let result = machine.run_until(end, inputs, &mut |event| {
    events.push(event);
    if matches!(event, Event::Infrared { .. }) {
        ControlFlow::Break(())
    } else {
        ControlFlow::Continue(())
    }
})?;
```

Implement `Output::event` for a custom consumer when it needs reusable state or methods.
`Vec<Event>` collects events, and `()` discards them. Return `Continue(())` to keep
running or `Break(())` to
return control after all effects at the current timestamp finish. Further events at
that timestamp are still delivered, and the entire input batch is consumed. The callback
is synchronous and must not re-enter the machine.

After a stop request, `RunResult.now` is the exclusive horizon one 64.64 time quantum,
`2^-64` seconds, after the completed instant. It can precede the requested `end`.
`RunResult.reason` is `StopReason::Output` when
the consumer requested a stop, including when the returned horizon equals `end`.
`StopReason::Horizon` means the call reached its requested horizon without a stop
request. Core faults return `Err` instead of a successful stop result. New
input may start at the returned horizon; the completed instant is already past. Resume
through another `run_until` call with the unconsumed inputs. The stop request belongs to
the current call and is absent from save states. Immediate power operations always
finish their complete operation and return.

A stop request is not per-event backpressure. The consumer must accept the remaining
events at that timestamp. If host delivery fails, retain the error and return
`Break(())`; check the retained error after the run returns. The device remains healthy.
Events already delivered to a callback are not emitted again on resume. Buffer them in
the application if the transport needs retries.

## Persistent updates and failures

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

## Display

`display(&mut [u8; 6144])` returns row-major 96x64 pixels. Each value is the
programmed PWM drive averaged over the selected FRC frames and scaled to 0..255,
rounded to the nearest integer. Zero is inactive and 255 is full drive. This includes
the programmable grayscale palette. Frontends can map these values to grayscale or
a panel tint and apply their chosen contrast response.

The image projects current RAM and programmed geometry. It does not retain earlier
scan rows or model the glass's response time. An inactive display or an undriven scan
clock produces zero pixels. `display_enabled` and `display_start_line` expose useful
controller state. `display_contrast()` returns the controller's electronic-volume
setting, 0..63. Read it alongside the pixels so firmware contrast changes can affect
the frontend's presentation. Pixel values describe duty; mapping this voltage control
to visible contrast requires the frontend's panel response.

`display_drive()` projects the LCD controller's digital output at the current
observation point: selected COM, 128 for the icon, two 64-bit SEG masks, and AC
polarity. It includes PWM/FRC, the output latch, and frame-latched start line. It does
not advance the guest or alter snapshot state. Power-save and display-off return
inactive drive. This projection contains no drive-voltage amplitudes, regulator or
follower behavior, or analog glass response. See the
[LCD accuracy contract](accuracy/lcd.md) for the modeled controls and limits.

## Audio

`machine.audio(sample_rate)` constructs an `Audio` renderer at the current time and
buzzer drive. Pass output events to `audio.event(event, samples)` and call
`audio.advance(result.now, samples)` after each run. Both accept a callback receiving
borrowed mono `i16` sample blocks. The renderer ignores other event types. The
[replay example](../crates/hs-core/examples/replay.rs) demonstrates streaming delivery.

The rate can be 1000 through 192000 Hz. Rendering uses band-limited synthesis with DC
removal, retains fractional phase across calls, and allocates only at construction.
Transition times round to 1/4096 of a sample. The filter introduces about eight samples
of delay. Output gain and the physical piezo's acoustic response remain presentation
choices; this converter does not claim to reproduce its measured sound pressure.

Rendering owns no machine state. Feed all buzzer events before advancing through their
interval, including a final advance through silence. After restoring or replacing the
machine, discard queued playback and construct a renderer for the new time and drive.
Sample/filter history belongs to the frontend and is absent from native save states.

## Stored bytes and inspection

`firmware`, `ram`, `eeprom`, `eeprom_status`, `sensor_nonvolatile`, `lcd_ram` and
`lcd_icons` return read-only data. RAM and EEPROM edits use the operations below.

`firmware()` returns an owned 48-KiB image of the current flash cells, including the
physical progress of an unfinished pulse. It may differ from the firmware loaded at
construction. The CLI exports this as `flash.bin`; the input file is never modified.
Exporting or peeking at flash does not perform a guest read, trigger protection or
finish a pulse. A raw image preserves readable bytes; an exact snapshot additionally
preserves intermediate cell charge and controls.

`registers`, `instruction_pc`, `phase_name`, `statistics`, `retired` and
`interrupt_entries` provide diagnostic state. `peek(address)` is a projection
with no guest read side effects. It may clone MCU state, so frequent inspection can be
expensive. These operations do not advance the device or emit events.
`phase_name` returns an implementation diagnostic; its exact labels may change.
Use `sleeping` and run results to control execution.

Instruction, interrupt, bus and serial totals accumulate across hardware resets. A
snapshot excludes these diagnostic totals; restoration starts them at zero. Use
differences between observations to measure a run interval.

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

[mgba-access]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/include/mgba/core/core.h
[sameboy-access]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/gb.h

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
TEST low at external reset release selects manufacturer boot mode; TEST high selects
a test mode. These modes return `Error::UnsupportedResetMode`. User mode executes the
supplied flash image. See the reset inputs in [INPUTS](INPUTS.md).

`Input::AnalogPin { pin, millivolts }` and `Input::DigitalPin { pin, level }` accept
optional fixture drives, where `None` releases them. `AnalogPin` and `DigitalPin`
name the package nodes. See [INPUTS](INPUTS.md#physical-csv) for the CLI equivalents.
These advanced electrical fixtures flow through the selected pin functions and their
owning peripherals. Ordinary frontends supply board inputs such as buttons, acceleration
and incident light.

Supply changes affect the analog network and functional availability. Falling below a
chip's operating range freezes or interrupts its physical work; this does not by itself
assert a clean brownout reset. RES charge and retention are separate mechanisms. Their
nominal constants and worked examples are in
[the power contract](accuracy/power-and-reset.md).

## Concurrency and allocation

Each run holds exclusive mutable access to its machine. Independent instances can run on
separate host threads. There are no global hardware variables or internal locks. A
caller may allocate an output vector; the core itself does not allocate during ordinary
execution. Construction, capture, encoding and some inspection operations allocate.
Their cost belongs outside the frontend's ordinary run loop.

Flash construction reserves at most 384 charge-page slots (about 3 MiB of address
space); a slot is populated only when its page is exposed to a pulse. Normal firmware
uses direct byte-array reads. Snapshot cloning preserves that reserve so even later
custom firmware programming does not allocate in a run.
