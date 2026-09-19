# Hardware accuracy

HachiStep models the Pokéwalker for ordinary use and custom firmware development. This
document describes the behavior the implementation supports, the evidence behind it,
and the limitations that affect reliance on its results. The hardware notes explain
individual mechanisms; the [source catalogue](SOURCES.md) identifies the documents,
firmware, code and investigations used to understand them.

Hardware knowledge and implementation coverage are separate. A documented effect can
still be missing from the implementation. An implemented behavior can have a strong
basis in firmware and circuit reasoning even when its datasheet omits the detail.
The sections below identify the particular remaining question and its consequence.
Supported behavior does not require a new physical measurement to become useful.

This describes the current source tree. Update an affected section when behavior or
its supporting evidence changes. Preserve the reasoning in the linked hardware note;
use issues for investigation and implementation discussion, and commits for change
history. A resolved limitation becomes a description of supported behavior here.

## Finding an answer

| Area | Current understanding and limits |
| --- | --- |
| CPU | [Instructions, accesses and exceptions](#cpu-instructions-accesses-and-exceptions) |
| Address space | [Memory, registers and GPIO](#memory-registers-and-gpio) |
| Time | [Clocks and operating modes](#clocks-and-operating-modes) |
| Counters | [Timer B1 and Timer W](#timer-b1-and-timer-w), [RTC](#rtc), [watchdog](#watchdog), [AEC](#asynchronous-event-counter-and-pwm) |
| Analog inputs | [ADC and battery sensing](#adc-and-battery-sensing), [comparators](#comparators) |
| Serial interfaces | [SSU](#ssu), [SCI and IrDA](#sci-and-irda), [IIC2](#iic2) |
| Motion | [BMA150](#bma150-accelerometer) |
| Display and sound | [NT7508](#nt7508-display-controller), [buttons and buzzer](#buttons-and-buzzer) |
| Persistent memory | [M95512](#m95512-eeprom), [internal flash](#internal-flash) |
| Board | [Infrared](#infrared-transceiver-and-connections), [supply and reset](#supply-reset-and-retention) |
| Whole machine | [Interactions and restoration](#interactions-and-restoration), [validation](#what-validation-establishes) |

The clearest known omissions are LCD supply/voltage controls whose effects are not
represented, sensor calibration fields without a transfer function, and the optical
transceiver's analog behavior. Other important questions concern the chosen battery
sensing circuit, startup and retention parameters, interrupted nonvolatile writes,
and timing during live reconfiguration. These have different consequences and should
be investigated according to the behavior they can change.

## CPU instructions, accesses and exceptions

The H8/300H interpreter supports the target instruction families, register widths,
addressing forms, arithmetic flags, branches, stack operations and exceptions. The
decoder applies the H8/38606 instruction set rather than admitting the extra H8S/H8SX
instructions in shared toolchain tables. RAM and flash execute through the same CPU.
Unassigned encodings produce a host decode diagnostic; the model does not establish
their physical outcome.

Execution retains instruction prefetch, partial operations and physical access order.
This covers writes to prefetched RAM code, source/address aliases in predecrement
stores, normal-mode addressing, discarded fetches and exception stack writes.
Interrupt handling includes reset's first-instruction deferral, enable-write races,
CCR instruction deferral and EEPMOV.W's NMI boundaries. Target manuals, Renesas Q&A,
GNU assembler encodings and independent MAME code support the interpretations.

Some source conflicts have specific resolutions: sticky SUBX Z follows the target
flag table; DAA includes reachable decimal states omitted from the printed table;
MOV.L stores accept the alternative selector supported by the conflicting manual and
toolchain encodings. Those resolutions are documented individually. Division by zero
retains the destination, overflow retains truncated result fields, and unspecified
decimal H/V flags retain their old values. The documented flags have a firmer basis
than these selected result bits. Firmware depending on unspecified results or
unassigned encodings needs further investigation.

See [execution](research/h8-execution.md), [encoding and arithmetic](research/h8-encoding-and-arithmetic.md),
[CPU state](CPU_STATE.md), and the [CPU implementation](../crates/hs-core/src/cpu/).

## Memory, registers and GPIO

The target map contains 48 KiB flash and 2 KiB RAM. Normal-mode effective addresses
truncate to 16 bits; word alignment, longword decomposition and register access widths
follow the physical bus. Register masks, reset values, read side effects and protected
writes are implemented by the relevant peripheral. GPIO resolution includes direction,
output latches, open-drain selection, pull-ups and peripheral pin priority.

The fixed board connects the shared serial bus, chip selects, sensor interrupt,
buttons, buzzer, battery sensing and infrared pins. Selecting an alternate function
on a connected pin can affect another device. The BMA150's I²C uses its existing
P91/P92 wiring; the MCU's IIC2 uses P90/P91. They are different bus connections.

Selected behavior remains for holes, prohibited access widths, reserved mux selectors
and PCR readback. In particular, PCR readback preserves direction latches, consistent
with the matching firmware's read/modify/write sequences despite write-only wording.
Conflicting EEPROM and sensor MISO drivers resolve low. The cited drive strengths
support that nominal choice but do not determine contention voltage or damage.
Analog fixtures use Vcc/2 as their digital threshold. Firmware relying on floating
pins, marginal levels or prohibited accesses reaches these model choices.

See [registers and GPIO](research/h8-registers-and-gpio.md),
[board evidence](SOURCES.md#board-and-firmware), and
[GPIO implementation](../crates/hs-core/src/mcu/gpio.rs).

## Clocks and operating modes

The model retains oscillator lifetimes, source phase, shared prescalers and peripheral
clock obligations. Active, sleep, watch, subactive, subsleep and standby apply their
documented clock and retention rules. Direct transitions pass through the intermediate
mode and count stabilization in oscillator cycles. A stopped source suspends its
remaining work. Clock outputs pass through the pin mux and can affect connected chips.

Renesas's target manual and A287/A333 amendments, together with `pw` clock transitions,
support the clock tree and mode behavior. Startup uses selected delays from electrical
tables. An electrical maximum used as the nominal delay does not establish when an
actual oscillator first supplies usable edges. Divider reset polarity, mux-induced
edges and some reconfiguration phases remain local circuit inferences. Review these
when a sequence depends on the first edge after reset, wake or a source change.

See [clocks and SCI](research/h8-clock-and-sci.md),
[startup](research/power-and-reset.md), and [clock implementation](../crates/hs-core/src/mcu/clocks/).

## Timer B1 and Timer W

Timer B1 implements interval/reload counting, load/counter aliasing, source selection,
overflow requests and retention across the applicable power modes. Timer W implements
counting, compare/capture, buffering, output modes, external inputs and the documented
input pipelines and access conflicts. The clocks and resolved pins supply their edges.
Firmware's Timer W initialization provides an independent example of the watch-clock
configuration used for timing.

Live source, mode and load changes retain progress under local rules where the manual
prescribes stopping first. Review same-time capture/compare/register accesses and
clock or pin changes against the manual's conflict diagrams. This is an interaction
review target, not a claim that these supported operations are absent. The additional
Timer B1 application note in the catalogue supplies a useful independent reload example.

See [counter rules](research/h8-counters-and-adc.md),
[Timer B1](../crates/hs-core/src/mcu/timer_b1.rs) and [Timer W](../crates/hs-core/src/mcu/timer_w.rs).

## RTC

Calendar counting, the alternate free-running counter, quarter/half-second events,
interrupt timing selection, clock output and reset-domain distinctions are implemented.
RTC data and controls survive MCU RES/watchdog reset where specified. Software RTC
reset has its own effects. A pending calendar update and the busy interval survive
suspension and save/restore.

The manual and same-target RTC application note specify approximately 62.5 ms between
busy assertion and the data update. HachiStep uses 512 watch/4 ticks, placed at ticks
7680 through 8192 of its second. The initial placement is inferred. Live writes during
busy interact with a latched prospective update; malformed BCD follows digit counters.
These choices matter for firmware intentionally racing an update or using invalid
calendar fields. Ordinary stable reads and calendar rollover have direct support.

See [RTC reasoning](research/h8-counters-and-adc.md#rtc-retained-state-busy-interval-and-raw-counters)
and [implementation](../crates/hs-core/src/mcu/rtc.rs).

## Watchdog

The watchdog implements protected register writes, count sources, interval/reset modes,
overflow flag qualification and its 512-ROSC reset hold. The hold combines with RES
qualification rather than releasing an outstanding reset. Source and prescaler lifetime
follow the operating mode. The matching firmware exercises the disable/service sequences.

Revision B of TN-H8*-A309 describes an instruction-address-dependent register-write
defect. The implementation applies it to the relevant MOV.B absolute-8 writes and
fields. This is a silicon rule attached to the actual access. Review unusual instruction
forms and simultaneous overflow/clear/reset against this erratum and the manual before
changing their behavior. A passing ordinary service sequence covers only one use.

See [watchdog reasoning](research/h8-counters-and-adc.md#watchdog)
and [implementation](../crates/hs-core/src/mcu/watchdog.rs).

## Asynchronous event counter and PWM

AEC supports independent/cascaded counters, external edge counting, PWM gates and
outputs, overflow requests and IRQAEC. External counting can continue with CPU clocks
stopped where specified. PWM period and low time use register value plus one; duty
greater than or equal to period forces the output low. Live changes traverse the
same gate and pin logic as clock-driven changes.

The exact miscount after a prohibited cascade-enable sequence, synchronizer aperture
and some live PWM transitions remain inferred. Documented restrictions explain why
these sequences deserve review, but do not themselves specify the missing result.
The manufacturer's AEC PWM example gives an additional period/duty configuration to
compare with the current model.

See [AEC reasoning](research/h8-counters-and-adc.md#aec-active-reconfiguration)
and [implementation](../crates/hs-core/src/mcu/aec.rs).

## ADC and battery sensing

The ADC implements channel selection, sample/hold, conversion work, result alignment,
completion requests, cancellation and external triggering. AVCC supplies its reference.
The nominal transfer rounds at half-LSB boundaries and clips to the ten-bit range.
The manual and subclock application note support conversion lengths and power behavior.

The acquisition aperture occupies four of 31 converter steps as a selected placement.
Live channel/clock changes, premature conversion after module enable and a disconnected
input use the retained capacitor and conversion state. These choices affect fast analog
changes and sequences outside the prescribed initialization procedure.

The board's battery circuit is less well established than the ADC transfer. `pw`
switches P84, samples PB3/AN3 and compares the result against EEPROM calibration.
HachiStep models that path as battery voltage minus a nominal 600 mV drop. Firmware
establishes the sequence and response direction, but not that circuit or drop.
Consequently a predicted battery-warning voltage has weaker support than the ADC code
for an explicitly supplied pin voltage. Component identification and existing board
evidence may refine this without changing the ADC itself.

See [ADC rules](research/h8-counters-and-adc.md#adc),
[battery circuit](research/adc-board-transfer.md), [ADC](../crates/hs-core/src/mcu/adc.rs)
and [board analog routing](../crates/hs-core/src/machine.rs).

## Comparators

Both comparators implement internal ladder and external reference selection, hysteresis,
settling, result latches, read-armed interrupt baselines and separate vectors. Renesas's
internal/external-reference examples resolve conflicting manual prose. A comparator
can continue while CPU clocks stop, subject to its own enable and power.

The model uses the specified 15 µs maximum as its nominal response time. Short transients
are filtered by that response model. Gate restoration starts a fresh analog response;
external reference with the prohibited hysteresis bit selects the external threshold.
Those transient and off-sequence behaviors are circuit inferences. `pw` provides no
reached comparator configuration to strengthen them. Timing-sensitive custom firmware
and narrow analog pulses are the relevant review cases.

See [comparators](research/h8-comparators.md)
and [implementation](../crates/hs-core/src/mcu/comparators.rs).

## SSU

SSU retains shift and holding state, phase/polarity, bit order, chip-select arbitration,
receive-only operation and slave/bidirectional modes. Partial transfers survive clock
changes according to remaining source edges. Completed bytes reach the actual EEPROM,
sensor and LCD parsers. `pw` supplies working configurations, including slow sensor
reads followed by faster bus use.

Live mode/order changes outside the prescribed stop sequence follow a retained-shift
model. Their exact transition boundaries remain inferred. Review these alongside pin
priority and device read side effects. Manufacturer SSU examples can test sequencing
independently of retail. The H8SX SSU erratum in the catalogue concerns other parts;
it has not established a corresponding H8/38606 defect.

See [SSU reasoning](research/h8-ssu.md), [SSU](../crates/hs-core/src/mcu/ssu.rs)
and [board serial synchronization](../crates/hs-core/src/machine/serial.rs).

## SCI and IrDA

SCI supports asynchronous and synchronous transfers, internal/external clocks, the
corrected five-bit formats, parity/framing/overrun behavior, receive-only master clocking,
holding registers and pin selection. TX completion status is distinct from completion
of the outgoing stop interval. Status-read qualification and RDR effects follow the
manual. IrDA encoding/decoding operates on timed signals through this SCI engine.

The A333B update removes multiprocessor operation and defines the five-bit formats.
Those changes apply to this target. The IrDA drawing leaves pulse launch phase and
decoder internals incompletely dimensioned; the model uses a centered pulse and a
retriggerable receive hold feeding the UART sampler. Live format changes also have
selected boundaries. These choices can affect marginal pulse widths, unusual clock
changes and peers with tight timing tolerances.

See [SCI and IrDA reasoning](research/h8-clock-and-sci.md)
and [implementation](../crates/hs-core/src/mcu/sci.rs).

## IIC2

IIC2 implements master/slave transfer, addressing/general call, ACK/NACK, stretching,
arbitration, receive continuation, filtered pins, status qualification and synchronous
serial mode. Resolved SCL/SDA levels feed back into the controller. Register requests
produce START/STOP through those pins. SSU and IIC2 share an interrupt vector and pin
priority follows the package diagrams.

The model incorporates the applicable reset, STOP, synchronization and receive-hold
errata. Equal nominal clock halves and exact collision windows for some defects are
inferred where only their conditions and consequences are documented. A defect's
presence can therefore be supported more strongly than the exact emulated race window.
Multi-master contention, same-edge reads and live CKS changes remain valuable targeted
review cases. The source catalogue distinguishes these errata from general I²C advice.

See [IIC2 reasoning](research/h8-iic2.md)
and [implementation](../crates/hs-core/src/mcu/iic.rs).

## BMA150 accelerometer

The sensor models range and bandwidth selection, sequential temperature/axis conversion,
filter history, offset calibration, protected registers and nonvolatile configuration.
New-data, low/high-g, any-motion and alert behavior use the sampled sensor state.
Sleep, autonomous wake, soft reset and self-test have their own transitions.
SPI and I²C share registers and read/shadow effects. The datasheet, Bosch-authored
drivers, ASF code and matching firmware support these mechanisms.

The model uses a rational conversion clock, a moving-average digital filter and a
second-order 1500 Hz analog response with selected Butterworth damping.
Conversion phase after wake, filter rounding/history during
reconfiguration, interrupt ties, read acknowledgement edges and some self-test timing
are inferred. Motion close to a threshold or a sleep/wake boundary can depend on them.
They should be reviewed as sensor behavior, with firmware's resulting steps as one
useful consequence.

The sensor represents a nominal healthy unit. Self-test completion assumes that unit
passes; it does not diagnose a simulated mechanical failure. The model does not add
random sensor noise, cross-axis sensitivity or an established temperature dependence
for acceleration gain/offset. These limits matter when estimating physical margins
around a threshold. Temperature input still drives the temperature conversion.

Gain trim, temperature trim and some protected calibration bits are retained without
a numerical transfer function. Firmware writes protected register 0x1e during normal
initialization; preserving that access has strong support, while its unestablished
analog effect remains a gap. Bosch's referenced ANA016/BST-MAS-AN014-01 has not been
recovered. Later calibration documents for different parts do not establish the scale.

On the board, CSB high permits sensor I²C on P91/P92, at fixed address 0x38. Sleep ACK,
pointer retention across interface changes and partial-read side-effect boundaries
include local inferences. No separate sensor register image is maintained for I²C.

See [sensor behavior](research/bma150-behavior.md), [sensor I²C](research/bma150-i2c.md)
and [implementation](../crates/hs-core/src/devices/bma150/).

## NT7508 display controller

The controller implements serial command/data parsing, RAM and icon storage, addressing,
geometry, mapping, palettes, PWM/FRC, scan phase and inversion. Software reset, RESETB
and power save have different effects. The display API supplies interpreted pixels and
digital scan drive. Oscillator frequency/control settings affect the scan. The
datasheet and `pw` initialization/fill routines establish geometry and plane order.

There is a known implementation gap in electrical controls. Converter/regulator/follower
enables, resistor ratio, booster, bias, temperature slope and contrast trim are stored
but do not affect exposed drive or the basic contrast value. The datasheet defines
effects for these controls. Firmware changing them can therefore produce an inaccurate
display result. Review the supplied voltage model and frontend information together;
the frontend should receive interpreted hardware output.

OTP control is also stored without a fusing model. Fusing requires an external programming
voltage whose delivery on this board is not established. Determine reachability before
adding a programming interface. External OSC1 selection stops internal advancement;
the board has no established external OSC1 driver.

RAM latch granularity, geometry changes during scan, some command/scan ties and restart
phase use explicit inferences. Pixel intensity expresses normalized programmed drive;
it is not a calibrated brightness curve. Glass persistence and optical appearance are
presentation work when a frontend requires them. Existing supply-control omissions
remain hardware questions regardless of that future presentation work.

See [LCD reasoning](research/lcd-and-eeprom.md),
[controller](../crates/hs-core/src/devices/nt7508.rs) and [scan](../crates/hs-core/src/devices/nt7508/scan.rs).

## Buttons and buzzer

Button inputs reach board pins and the same GPIO/interrupt/analog routing used by guest
accesses. Firmware performs polling and debounce. Timer/GPIO output determines piezo
drive, and timed buzzer events continue whether an application plays audio or mutes it.
The reusable audio converter produces PCM from these events.

The input level history is supplied by the application. Mechanical contact bounce is
represented only when that history includes it. Audio conversion represents drive
timing, not a calibrated piezo, enclosure resonance or sound-pressure level. These
limits matter for physical acoustics and marginal contact behavior. They do not require
an interactive frontend as a prerequisite for reviewing the core's pin and timing rules.

See [GPIO evidence](research/h8-registers-and-gpio.md),
[presentation rationale](research/emulator-presentation.md),
[signals](../crates/hs-core/src/signals.rs) and [audio](../crates/hs-core/src/audio.rs).

## M95512 EEPROM

The device implements the 64 KiB array, 128-byte page behavior, serial commands,
WEL/WIP, status protection and asynchronous internal writes. Chip-select completion
and power interruption affect the device operation; firmware performs checksums,
mirroring and recovery. Save states retain the pending write as well as the cells.

The board investigation identifies M95512RP. The catalogue includes the R datasheet,
family revisions and vendor application notes. Verify the installed generation when
using later voltage, timing or extra-feature descriptions. A newer family's identification
page is not evidence that this board has one.

The component's write-protect input exists in the model but defaults to released;
the machine does not route a board signal to it. HOLD has no implemented serial-pause
input. Establish whether either pin is controllable on the actual board before adding
a route. Their datasheet presence alone does not make them firmware-accessible.

Interrupted programming uses a deterministic erase/program progression, including
same-value writes and writable status cells. The half-duration phase split and cell
threshold schedule are selected approximations. They permit recovery experiments but
do not predict the exact corrupted bits from a particular physical power failure.
ST's EEPROM architecture and power-on-reset notes are useful further sources for this
model. Wear and long-term retention degradation are not established by this progression.

See [EEPROM reasoning](research/lcd-and-eeprom.md#m95512-concrete-protocol-rules)
and [implementation](../crates/hs-core/src/devices/m95512.rs).

## Internal flash

The flash model uses the target's six erase blocks and 128-byte program latch. Guest
instructions select setup, pulse and verify operations. Protection, settling and
interrupted exposure affect the same array from which the CPU fetches. Firmware owns
retry loops and programming algorithms. Retail's external EEPROM driver does not
validate these internal-flash operations.

Normal reads and verify reads use distinct cell thresholds. The selected distribution
and exposure scale are based on aggregate timing and the documented programming
mechanism, without a measured per-cell distribution. This supports partial-progress
behavior but limits predictions of exact interrupted bits and marginal pulse success.
Review source timing bounds, protection transitions and the justification for the
distribution before expanding its complexity.

Construction supports ordinary user-mode reset. Manufacturer/test strap selections
return `UnsupportedResetMode`; no manufacturer ROM program is substituted by the core.
That boundary is separate from guest-controlled flash hardware. Unavailable ROM contents
do not justify implementing their software procedures as peripheral behavior.

See [flash reasoning](research/h8-flash.md)
and [implementation](../crates/hs-core/src/mcu/flash.rs).

## Infrared transceiver and connections

The board interface emits timed optical levels and accepts incident levels through
the SCI/GPIO path. P30 controls shutdown, P31 receives active-low input and P32 controls
emission. These connections are supported by matching firmware sequences. Applications
transport signals between independently running devices; firmware implements discovery,
packets, retry timing and game-specific exchanges.

The current board path uses digital gating and polarity. It does not implement a
characterized optical receiver's pulse shaping, turnaround recovery, ambient-light
response or sustained-transmit cutoff. The actual transceiver remains unidentified.
ROHM and Vishay documents establish plausible mechanisms and useful comparisons;
their specific timings and self-echo behavior are not established Pokéwalker properties.

Emulated walker-to-walker exchange exercises the firmware and signal interface.
It does not by itself establish communication with an HGSS cartridge or a physical
walker. Those peers can expose timing or optical assumptions shared by two identical
emulated devices. Component identification and existing independent communication
evidence are useful next sources.

See [infrared evidence](research/infrared.md),
[application boundary](research/emulator-linking.md) and [board implementation](../crates/hs-core/src/machine.rs).

## Supply, reset and retention

Supply changes reach the MCU and external chips. The model distinguishes reset,
clock startup, chip availability, volatile retention and interrupted persistent writes.
RES release and watchdog reset combine, and external devices can progress while the
CPU is held. MCU reset does not automatically cold-reset every peripheral chip.

The common battery-derived rail follows the available board and firmware evidence.
Reset charging uses a nominal RC response; volatile loss uses an accumulated
low-voltage exposure. These parameters, deterministic cold contents and chip availability
thresholds are approximations rather than a reconstructed complete analog circuit.
Some startup delays use datasheet maxima. The model does not calculate a CR2032's
discharge curve or load-dependent rail droop; supplied voltage describes those conditions.

RES2B/VCI pad identities and peripheral reset connectivity remain incompletely known.
The selected MCU-only external reset must not silently become a common reset net based
on a pad label. Exact short-collapse retention and marginal-voltage operation need
stronger board/component evidence. Ordinary digital reset domains have more direct
support than these analog boundaries.

See [power and reset](research/power-and-reset.md),
[power implementation](../crates/hs-core/src/power.rs) and [startup clocks](../crates/hs-core/src/mcu/clocks/startup.rs).

## Interactions and restoration

Shared pins, interrupt requests and clock sources connect the device models. Arithmetic
advancement preserves the state needed for later observable effects. Execution horizons
and save/load retain partially completed CPU accesses, serial frames, timer phase,
sensor history, scan state and nonvolatile programming. Firmware-independent RAM and
EEPROM edits are explicit host operations.

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

See [state semantics](SAVE_STATES.md), [embedding](API.md),
[execution](../crates/hs-core/src/machine/execution.rs) and [state handling](../crates/hs-core/src/machine/state.rs).

## What validation establishes

[hachiware](https://github.com/lumirth/hachiware) owns independent guest diagnostics,
fixtures and expected observations. Its case definitions explain the evidence for
each expectation. Relevant families can be found directly in its
[cases directory](https://github.com/lumirth/hachiware/tree/main/cases).

| Behavior | Diagnostic families |
| --- | --- |
| Instructions and accesses | `cpu.py`, `decimal_adjust.py`, `bus.py`, `interrupts.py` |
| Clocks, counters and analog | `clocks.py`, `timers.py`, `rtc.py`, `watchdog.py`, `adc.py`, `comparators.py` |
| Serial hardware | `ssu.py`, `serial.py`, `iic.py` |
| Connected devices | `sensor.py`, `sensor_i2c.py`, `lcd.py`, `eeprom.py` |
| Persistent operations and supply | `flash.py`, `power.py` |

Local checks also exercise partition independence, save/restore, API contracts and
target consistency. Retail scenarios cover startup, menus, motion, game activities,
time progression, persistent writes and peer exchange. Their reviewed output baselines
protect against regressions in those scenarios. They do not provide independent
physical measurements of every effect they contain.

For a claim about hardware, inspect the expected behavior and its basis. For a claim
about restoration, compare subsequent observations and retained state. For a claim
about performance, use representative workloads whose behavior remains equivalent.
[TESTING](TESTING.md) provides the commands and evidence limits; individual run results
belong with the run or change rather than in this hardware reference.
