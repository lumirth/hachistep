# Interactions and restoration

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

Shared pins, interrupt requests and clock sources connect the device models. Arithmetic
advancement preserves the state needed for later observable effects. Execution horizons
and save/load retain partially completed CPU accesses, serial frames, timer phase,
sensor history, scan state and nonvolatile programming. Firmware-independent RAM and
EEPROM edits are explicit host operations.

## Limits and open questions

Review interactions when a change affects the next observable boundary. Useful cases
include a register write coinciding with an interrupt, selecting a clock output on a
chip-select pin, a sensor interrupt during a slow serial read, two MISO drivers,
and reset or supply loss during an access. Existing component coverage does not
establish every combination. These are focused review questions, not a demand to
enumerate every possible machine state.

Native save states preserve the modeled machine. Audio playback buffers and frontend
presentation history have their own lifetimes. Restoring the same modeled behavior is
a separate property from whether that behavior matches the physical device. The format
remains changeable before release; there is no current interoperability guarantee.

## Review boundaries

Component checks establish particular operations under particular conditions. The
machine must also preserve effects when components interact. The following boundaries
already have relevant checks and still warrant attention when their owners change.
They identify the scope of review, rather than asserting an undiscovered defect.

| Boundary | Existing evidence or checks | Remaining scope to consider |
| --- | --- | --- |
| A guest access coincides with a device event. | CPU admission, register-access and Timer W conflict tests exercise specific ordering rules. | Each peripheral has its own precedence; a global ordering convention cannot replace a documented exception. |
| A source changes with work in progress. | Clock-obligation tests cover ADC, SSU and oscillator transitions; machine tests preserve serial histories across partitions. | Live format changes and peripheral-specific latch phases still include the inferences recorded in their topics. |
| A mux changes a connected pad. | RTC clock-output, SSU overlapping-read, IIC and sensor I²C cases exercise actual board routing. | Combinations involving another active alternate function can expose a connection missed by isolated tests. |
| Reset or supply loss interrupts work. | Power, watchdog, flash and EEPROM cases distinguish reset domains and ongoing operations. | Analog availability, first edges and damaged-cell values retain the limits of the selected models. |
| An application stops, edits or restores a machine. | Output-control, state-editing and save-state tests exercise the public contracts. | Complete histories and subsequent behavior matter; an identical final framebuffer or memory block is insufficient. |

For a suspected error, identify the guest-visible effect and follow its actual owners:
source clock, input or bus access, device transition, pin/interrupt propagation, and
observation. This keeps investigations attached to behavior. A source disagreement may
require better interpretation; a missing connection may require implementation even
when every component rule is already known.

## Host time, edits and physical input

The application chooses how much emulated time to run and supplies timestamped inputs.
Host pacing does not change the relationship between CPU and peripheral clocks.
Signals between independently running machines retain their emulated timestamps;
transport and execution scheduling belong to the application. The core exposes the
hardware connection, while firmware performs the link protocol.

An application can edit RAM or persistent storage through the public state-editing
operations. Such an edit deliberately changes the machine's history. A frontend that
adds steps using knowledge of a firmware layout cannot infer all the gifts, timers or
other effects that running that firmware through an elapsed interval would have caused.
Similarly, reconstructed motion is a supplied trajectory, not proof of the motion a
physical walker experienced. These application choices do not select a different
hardware model. [Embedding and editing](../API.md), [state-editing checks](../../crates/hs-core/tests/state_editing.rs).

## What validation establishes

[hachiware](https://github.com/lumirth/hachiware) owns independent guest diagnostics,
fixtures and expected observations. Its case definitions explain the evidence for
each expectation. Relevant families can be found directly in its
[cases directory](https://github.com/lumirth/hachiware/tree/main/cases).

| Behavior | Diagnostic families |
| --- | --- |
| Instructions and accesses | [CPU](https://github.com/lumirth/hachiware/blob/main/cases/cpu.py), [decimal](https://github.com/lumirth/hachiware/blob/main/cases/decimal_adjust.py), [bus](https://github.com/lumirth/hachiware/blob/main/cases/bus.py), [interrupts](https://github.com/lumirth/hachiware/blob/main/cases/interrupts.py) |
| Clocks, counters and analog | [Clocks](https://github.com/lumirth/hachiware/blob/main/cases/clocks.py), [timers](https://github.com/lumirth/hachiware/blob/main/cases/timers.py), [RTC](https://github.com/lumirth/hachiware/blob/main/cases/rtc.py), [watchdog](https://github.com/lumirth/hachiware/blob/main/cases/watchdog.py), [ADC](https://github.com/lumirth/hachiware/blob/main/cases/adc.py), [comparators](https://github.com/lumirth/hachiware/blob/main/cases/comparators.py) |
| Serial hardware | [SSU](https://github.com/lumirth/hachiware/blob/main/cases/ssu.py), [SCI](https://github.com/lumirth/hachiware/blob/main/cases/serial.py), [IIC](https://github.com/lumirth/hachiware/blob/main/cases/iic.py) |
| Connected devices | [Sensor](https://github.com/lumirth/hachiware/blob/main/cases/sensor.py), [sensor I²C](https://github.com/lumirth/hachiware/blob/main/cases/sensor_i2c.py), [LCD](https://github.com/lumirth/hachiware/blob/main/cases/lcd.py), [EEPROM](https://github.com/lumirth/hachiware/blob/main/cases/eeprom.py) |
| Persistent operations and supply | [Flash](https://github.com/lumirth/hachiware/blob/main/cases/flash.py), [power](https://github.com/lumirth/hachiware/blob/main/cases/power.py) |

Local checks also exercise partition independence, save/restore, API contracts and
target consistency. Retail scenarios cover startup, menus, motion, game activities,
time progression, persistent writes and peer exchange. Their reviewed output baselines
protect against regressions in those scenarios. They do not provide independent
physical measurements of every effect they contain.

For a claim about hardware, inspect the expected behavior and its basis. For a claim
about restoration, compare subsequent observations and retained state. For a claim
about performance, use representative workloads whose behavior remains equivalent.
[TESTING](../TESTING.md) provides the commands and evidence limits; individual run results
belong with the run or change rather than in this hardware reference.

## Implementation and checks

The [machine executor](../../crates/hs-core/src/machine/execution.rs) coordinates
component work and [state handling](../../crates/hs-core/src/machine/state.rs) captures
and restores it. [Machine tests](../../crates/hs-core/tests/kernel.rs),
[output-control tests](../../crates/hs-core/tests/output_control.rs), and
[save-state tests](../../crates/hs-core/tests/save_state.rs) exercise chronology and
restoration. [API](../API.md) and [SAVE_STATES](../SAVE_STATES.md) own the public contracts.
The scope of hardware expectations still comes from the corresponding topic and source,
even when the resulting execution is exactly reproducible.
