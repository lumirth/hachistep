# Implementation status and fidelity boundary

The 0.2 development core is runnable and regression-tested. It is **not** a complete
implementation of the preceding design specification. In particular, there is
no claim that arbitrary custom firmware cannot distinguish it from silicon.

There is one production execution engine and one hardware model. There is no
retail-PC patching, firmware-function replacement, step injection, fast/accurate
mode, atomic-byte backdoor beside a pin-level serial implementation, or rollback
interpreter. Firmware and original EEPROM bytes are not edited to make boot pass.

## Evidence vocabulary

**Implemented/tested** means the listed mechanism runs and has the stated
software tests. **Witness** means a deterministic, plausible choice was needed
but the exact silicon rule/parameter has not been established. **Unsupported**
means the operation stops rather than pretending to implement it. A passing
fixture is not a physical measurement; no new physical captures were obtained.

Not every incomplete behavior has a trap. Storage-only/nominal witnesses are
listed explicitly below. An absence of model faults is not a completeness proof.

## Owner matrix

| Owner | Implemented/tested | Important limitation |
|---|---|---|
| `time.rs` | 64.64 seconds, retained rational remainder, checked arithmetic, exact inverse edge counting and partition tests. | Timestamp precision does not establish physical clock accuracy. Very large representable requests can still exceed the finite edge-ordinal domain and fail. |
| `cpu/decode.rs` | Incremental decoding of H8/300H normal-mode scalar, memory, branch, bit, control, multiply/divide, exception and block-copy forms; all 65,536 first words classify without panic; 192 independently stated displacement-24 valid/invalid cases. | Classification totality is not complete encoding certification. Invalid/reserved selector handling needs a full independently reviewed encoding corpus. |
| `cpu/alu.rs` | Explicit 8/16/32-bit arithmetic, register aliases, CCR updates; exhaustive byte add/sub input cases. Division continues on zero/overflow with documented N/Z flags; hachiware checks all signed/unsigned widths. | Undefined divide destination bits retain the dividend on zero and narrow quotient/remainder on overflow. Wider flag cases and unusual DAA/DAS incoming flags need broader independent evidence. |
| `cpu/mod.rs` | One resumable interpreter with retained opcode prefetch and ordered physical accesses. Independent diagnostics cover RAM self-modification, call/stack aliases, predecrement, RTE/LDC admission, EEPMOV interruption, reset's first instruction, enable-clearing admission and duplicated exception CCR bytes. | Subcycle NMI timing and some internal commitment phases remain inferred. STC.W's unspecified odd byte is zero. See [execution research](research/h8-execution.md) and the [CPU audit](research/h8-cpu-completion-audit.md). |
| `machine.rs` | Exclusive horizons, physical timelines, cached next-device boundary, callbacks, fault latching, snapshots, reset/power API. CPU, SSU, SCI, ADC and power-transition waits retain source-edge obligations. | Same-time ordering is currently devices, then input batch, then CPU; individual register conflict rules need further coverage. |
| `mcu/clocks.rs` | Separate CPU and peripheral references, shared oscillator phases, resettable S/W prescalers with retained stop phases and monotonic edge ordinals. SSU's subclock uses the latched SA divider. | Nominal 3,686,400 / 32,768 / 1,310,720 Hz are model choices. OSCCR stop/mux controls and ROSC consumer gating are implemented. Oscillator startup envelopes and feedback analog behavior remain physical-model work. No arbitrary rational frequency through public `Conditions` yet. |
| `mcu/control.rs` | Main/subactive/sleep/subsleep/watch/standby, direct-transition intermediate modes and one old-clock internal cycle, STS wait in undivided oscillator cycles, masked transitions, external IRQ/NMI. IRQ mux changes on low inputs set the request and protect its clear through one intervening instruction. | Mux clearing delay implements the documented instruction-level effect; it is not a characterized analog delay. The canonical board uses its main-oscillator reset strap. Bootstrap mode still needs an owner. |
| `mcu/gpio.rs` | Port latches/directions, SSU package mux and IRQ priority, independent serial selects, P9 open-drain release, physical buttons and timer drive routing. Pull-ups require input direction even when an alternate input is selected; comparator-enabled PB pins retain their digital reads. | Electrical contention and some alternate functions remain incomplete. Timer W FTIOA/B/C/D/FTCI are routed through actual package pins. PCR readback remains a latch witness. No invented BMA interrupt wire. |
| `mcu/timer_b1.rs` | Analytical interval/reload counting, distinct load state, overflow IRQ, gated clocks, live load/mode/source writes and mux-induced edges. | Accepting live writes and their mux edges follows the connected latch/counter model beyond the recommended stopped configuration sequence. |
| `mcu/timer_w.rs` | 16-bit compare-on-leaving, overflow, clear-on-A cycle, PWM, paired buffers, input capture, external FTCI, documented write/capture/buffer conflicts, stopped-counter capture and IRQ. | Input pipelines are reference-edge abstractions, not characterized sub-state propagation. Internal low-to-high clock-selector changes increment the counter. Prohibited byte accesses use the selected word-lane rule below. Buzzer edges still schedule individually; no compressed waveform law yet. |
| `mcu/rtc.rs` | Raw digit counters, binary counter, calendar aliases, periodic IRQs, latched busy update, live writes and retained reset domains. TMOW and CLKOUT drive P10 with actual divider edges, independently of RUN. | First busy phase, live-write collision precedence and malformed digit carries use the local rules in the counter research note. Prohibited CLKOUT selector 111 releases the output. |
| `mcu/watchdog.rs` | All selectors, retained private ROSC divider, old-latch MOV qualifications, OVF read qualification, interval IRQ, absolute-8 MOV alignment erratum, WRST and 512-ROSC reset hold; overlapping RES and mid-reset replay. ADC result and RTC retained reset domains are preserved. | Clear erratum for other MOV addressing forms follows the ordinary write rule pending a characterized timing rule. |
| `mcu/adc.rs` | 31 converter steps on all four clocks, held sample, live mux/clock writes, open-mux charge retention, two-stage ADTRG synchronization and vector 38, gate retention and ADRR reset retention; half-LSB quantization. | Four-step acquisition placement and premature-settling charge retention are circuit inferences. AVCC follows supply unless an external fixture overrides it. P84 high drive enables a nominal supply-minus-600-mV PB3 path; the effective drop and immediate off-state discharge are circuit inferences, not a measured board netlist. |
| `mcu/ssu.rs` | One shifter for master and external-clock slave TX/RX, all four SPI phases, bidirectional pin routing, hardware SCS arbitration/deselection, queued data, receive-only/overrun/single-stop sequencing, live CKS continuation and retained sequencer reset. SOL readback/protection and open-drain release are implemented; independent guest cases cover package-level transfers. | Internal pin synchronization is an edge abstraction. Exact active clock-mux glitches and some out-of-sequence configuration effects remain inferred. |
| `mcu/sci.rs` | One clock-counted shift/holding owner for asynchronous and synchronous TX/RX, internal/external clocking, corrected five-bit formats, startup mark, stop/D7 preload, sampled start detection, errors/overrun, and IrDA. GPIO and SCI share P30 shutdown, P31 receive, and P32 transmit. | IR pulse launch/decoder and off-sequence clock/format transitions use the local circuit rules in the SCI research note. Analog optical response remains incomplete. HGSS peer interoperability has not been tested. |
| `devices/m95512.rs` | Bit-level SPI commands, sequential reads, 128-byte page wrap, WEL through write completion, protection, exact WRSR length, POR selection qualification, and delayed commits. Power loss retains partial addressed cells; exports project the same cells without completing the write. | Five-millisecond programming time, equal erase/program phases and fixed per-cell thresholds describe the canonical part. Threshold distribution is inferred. Board HOLD/WP routes are not invented. |
| `devices/bma150.rs` | Physical micro-g input, quantization, bounded filter history, register/shadow state, protected window, working/nonvolatile image, four-wire SPI, basic data-ready/any-motion state. | Exact filter window mapping, rounding, calibration effects, staggered axis publication, interrupt algorithms and physical parameters are not certified. See details below. |
| `devices/nt7508.rs` | 4 KiB RAM plus icons, serial parser, persistent pending parameters across deselection, command/control state, addressing/bitplanes, logical 96x64 rendering. | Analog drive/FRC/PWM/scan timing and several retained analog control effects are not simulated. Panel COM mapping and terminal-column behavior are witnesses. |
| `mcu/flash.rs` | Target 48-KiB array and six erase blocks, register gates/protection, 128-byte page latch, cumulative programming/erase exposure, four-byte verify latch, reset/power retention and separate 20-µs wake. RAM-executed guest cases cover retry programming, both large erase blocks, FLER and early sense/wake reads. | Cell thresholds and setup/recovery behavior outside prescribed algorithms use the nominal physical model in [flash research](research/h8-flash-implementation.md). No wear or measured charge-pump voltage curve. Flash images project partial cells; per-pulse persistence callbacks are not yet emitted. Bootstrap execution remains separate work. |
| Comparators | Both channels; correct P30 VCref route, ladder/external references, hysteresis, read-armed IRQ baseline, qualified flag clear, vector 36, module/reset behavior; guest wake/replay test and pin stimuli. | Response uses a 15 µs inertial witness (the manual specifies a maximum, not an exact delay). Non-hysteresis VIH follows Table 18.2/Fig.18.2 despite conflicting CRS prose. No characterized analog noise/offset. Only the ADC-selected PB pin suppresses its digital read. |
| AEC | Independent 8-bit and cascaded 16-bit counters, external edge selection, analytical internal counts, gated clock-return edges, PWM gating/output, read-qualified overflow flags and separate vectors 18/32. Real P10/P11/P12 pin routing and digital fixture inputs; inactive work elision with tested rejoin to shared clock phases. | Reference-prescaler polarity, coincident gate/clock ordering, module-stop details and IRQ synchronizer aperture need physical characterization. Live period/duty/source writes retain counter phase, forced-low output traverses the shared gate, disconnected clock/edge selections park, and reserved writable fields retain values. Undefined ECPWDR reads return zero as the selected bus value. |
| `mcu/iic.rs` | One I²C/synchronous shifter, master/slave addressing, queued data, ACK/NACK, read-qualified flags, shared vector 34, two-sample pad filters, open-drain P90/P91 with SSU priority, arbitration, receive/transmit stretching, WAIT insertion and retained half-phi clock obligations. | Equal clock halves, live reconfiguration and the bounded A017/A023 collision windows are local circuit rules described in [IIC research](research/h8-iic2-implementation.md). Line capacitance and analog edge slew are not modeled. |
| Register-access contract | Independently transcribed width/cycle expectations; CPU-boundary tests distinguish two-state RTC/timers/ADC/SPCR/IrCR from three-state SSU/SCI. Mixed word accesses and wrapping long accesses retain separate physical cycles. | For prohibited byte accesses to word registers, reads select the readable lane and writes do not qualify the latch. Holes read zero and discard writes. These are compact bus inferences, not measured invalid-access outcomes; see [register research](research/h8-register-and-gpio-completion.md). |
| Frontend/persistence | No-clobber CLI, CSV, raw image exports/import, JSON reports, PGM, ideal-drive WAV tool, typed in-memory snapshot. | No GUI/live-link frontend, no portable snapshot encoding, no atomic multi-file save container. Host export is separate create-new files; report written last. |

## Specific witnesses that must not become invisible assumptions

### Protected BMA150 register 0x1E

Retail initialization reaches this address while protected access is enabled.
The owner models gating and low-level read/write retention. It does not claim
the internal physical meaning of every field at this address is understood.
The test named `protected_window_and_identity` tests the implemented access
contract, not a measured hidden calibration circuit.

### MCU address 0xF088

The matching firmware writes `3` during infrared initialization. Both inspected
manufacturer register maps and the target addition leave this address unassigned.
It therefore uses the ordinary hole rule: reads return zero and writes complete
without a stored latch or pin effect. The executed firmware write establishes an
access, not a hidden two-bit register contract.

### Sensor front end

Default input is stationary +1 g on Z at 20 °C. The factory image uses the
documented ±4 g/1500 Hz defaults; firmware normally selects ±2 g itself.
Temperature, X, Y and Z publish in successive 12 kHz slots. Axis input is
sampled at its own boundary; a Z publication requests new-data IRQ only when
all three freshness flags are set. The first complete cold-start vector is ready at 3 ms; normal wake uses 1 ms.

The filter retains 64 actual samples per axis with running sums. Bandwidth
selects 64/32/16/8/4/2/1 samples, arithmetic-shift floor rounding, and raw output
until the chosen window fills. Bandwidth changes reuse history. Offset-binary
trim acts relative to the fixed modeled factory midpoint 512, at +31.25 mg
per step, before signed ten-bit ADC clamping and filtering. Range and trim
changes therefore pass through existing filter history. Reserved range/filter
codes retain their register bits and use the widest/unfiltered realization.
The research note identifies the timing and rounding choices alongside their
Bosch evidence. Temperature follows code/2−30 °C and is an explicit input.

Low/high-g criteria retain per-axis hysteresis, millisecond debounce counters,
active status and independent latches. Any-motion uses three-interval differences
and qualifies both edges; alert shortens working durations without changing
configuration. Data-ready remains separate. The physical board IRQ connection
is not asserted without evidence. Autonomous wake retains its physical phase
separately from the programmed sleep bit, with all four pause lengths, filter
acquisition, interrupt verification, latch retention and a 330 µs minimum IRQ.
Soft reset restores working state and includes a 10 µs serial quiet interval.
Image reload blocks image/NV access until its 300 µs completion. Self-test 1
feeds zero ADC codes through the normal filter; self-test 0 holds the published
vector for one full conversion cycle and reports success for the modeled
healthy unit. That completion boundary is inferred; no deflection amplitude
is invented. Four-wire reads and three-wire
turnaround share the serial parser; only reads auto-increment. Three-wire drives
SDI and leaves SDO floating, with MCU sampling preceding the sensor's same-edge
output change. A launched but unclocked next byte has no read side effects.
I2C, complete
calibration effects, analog filtering and noise are not implemented.

### LCD viewport

The panel view uses SEG0–95 and COM32–95. Direction selection acts on the full
128-output controller before cropping; partial duty leaves inactive commons
blank. The first byte of each column is the high gray bit, consistent with
`pw`'s independently named fill patterns. Display-off overrides entire-on,
which overrides reverse. Icon enable and page selection are independent, and
only DB0 is stored in the icon page. Software reset retains RAM and the drive
controls listed as unaffected in the manual's specific reset table.

The digital scan retains oscillator/divider phase, a binary 128-SEG output
latch, row/PWM/FRC counters, frame-latched geometry/start line, and continuous
n-line inversion. Stable intervals advance arithmetically without scheduler
appointments. Palette nibbles select documented PWM widths; entire-on uses
the black palette. Display-off gates drive; power save stops scan; release
restarts at row zero with retained RAM. External OSC1 is undriven on the modeled
board. Live-change boundary choices are centralized in the cited research note.

`display_drive` exposes this digital drive; the existing shade raster remains
a logical RAM view. Voltage/contrast controls are retained, but calibrated
panel response, analog settling and physical luminance are not implemented.
Unassigned/factory-test bytes are inert in the chosen board model; E8 belongs
to the unbonded three-wire interface and does not consume a parameter here.

### Power

`Input::ResetPin` is a package-level MCU reset drive. Its low assertion is
asynchronous; raising it starts the documented eight-phi release counter.
Reassertion discards partial qualification, and WDT's independent 512-ROSC hold
does not gain a second release count. `Machine::power_off/on`
acts on the whole product. Input supply voltage affects the modeled battery measurement; a zero-voltage
interval also invokes board power loss/restoration. Nonzero voltage changes do
not invent a clean brownout reset.
Cold RAM/CPU unspecified values are initialized deterministically. NMI defaults high as the user-mode strap. A low NMI at reset release/power-on rejects unimplemented bootstrap modes rather than silently executing user firmware. The selected NMI edge is latched independently of IEN/IRR; short-pulse synchronizer behavior is not characterized. A zero-voltage rail or explicit power-off stops all activity and resolves partial
EEPROM/sensor writes. Completed writes emit NvCommit; interrupted writes emit
NvInterrupted after their persistent-byte observations. The shared cell model
uses zero as its erased state (documented for ST; inferred for Bosch), fixed
thresholds and equal erase/program phases. RES-capacitor discharge, chip-specific
undervoltage availability and cold-start readiness still need integration; a
minimum rated operating voltage is not treated as a clean reset threshold.
The new flash wake window also exposes a power-on integration gap: a caller
must currently hold RES through flash startup before raising it, or the early
reset-vector read sees unavailable data. The board startup/reset owner is the
next integration task; see the [flash audit](research/h8-flash-owner-audit.md).

## Performance boundary

The default engine has no ordinary-run allocations and no second executor.
Inactive AEC clock/pin work is now elided without replaying gated time; see the scoped paired measurement in REVISION-0.2. It still decodes on execution, schedules individual serial/buzzer edges,
and schedules BMA conversion phases at 12 kHz (3 kHz per axis). It does not yet implement all analytical
edge-run and signal-law optimizations in the design. No fastest-emulator or
mobile energy-efficiency claim is made. See actual samples in `evidence/`.

## Consequence for custom firmware

The implemented mechanisms can be exercised by custom images, including RAM
execution and independently assembled diagnostic programs. A complete clean run
is useful evidence but cannot establish that the firmware transfers correctly to
hardware outside the listed coverage. Consult this matrix, use focused fixtures,
and do not reinterpret stored-only witnesses as full peripheral emulation.

## Electrical-fixture inputs

`Input::AnalogPin` / CSV `time_us,analog,pb4,1900` supplies a voltage on an
actual PB0..PB5 or VCref package node; `release` returns to the board-derived
voltage. This is a test-fixture stimulus, **not a new user-accessible control on
an unmodified Pokéwalker**. PB4 remains the right-button net in ordinary use.
Unconnected PB1/PB5/VCref defaults are zero-voltage witnesses. Digital input
projection of an override uses Vcc/2, not characterized input thresholds. ADC
inputs use the documented midpoint quantizer against external AVCC, with the
nominal battery circuit described in [ADC board research](research/adc-board-transfer.md). All these inputs
are serialized in in-memory snapshots and validated before a batch mutates the
machine. CMOS contention, clamps and loading from forced voltages are not modeled.
