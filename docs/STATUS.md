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
| `cpu/mod.rs` | One resumable interpreter with a retained opcode prefetch; ordered NEXT, branch/call/return, bit-RMW, EEPMOV, reset and exception accesses from the H8 instruction tables. RAM self-modification and a stack aliasing a call target have independent guest diagnostics. Predecrement aliases, RTE/LDC admission and NMI interruption of EEPMOV are covered. | General request-sampling/enable races, subcycle NMI timing and unspecified CCR-word bits remain incomplete. Some internal effects occur at provisional phase boundaries. See [execution research](research/h8-execution.md) for conflicting table entries and the selected interpretation. |
| `machine.rs` | Exclusive horizons, physical timelines, cached next-device boundary, callbacks, fault latching, snapshots, reset/power API. CPU, SSU, ADC and power-transition waits retain source-edge obligations. | Same-time ordering is currently devices, then input batch, then CPU; individual register conflict rules need further coverage. SCI still uses captured frame periods. |
| `mcu/clocks.rs` | Shared oscillator phases and divider ordinals; switching joins the selected running source phase. SSU's subclock uses the latched SA divider. | Nominal 3,686,400 / 32,768 / 1,310,720 Hz are model choices. Prescaler S/W stop/reset rules and OSCCR source control still need completion. No arbitrary rational frequency through public `Conditions` yet. |
| `mcu/control.rs` | Main/subactive/sleep/subsleep/watch/standby, direct-transition intermediate modes and one old-clock internal cycle, STS wait in undivided oscillator cycles, masked transitions, external IRQ/NMI. | Full source-specific retention, oscillator controls and simultaneous admission races are not complete. Reset-mode bootstrap straps remain incomplete. |
| `mcu/gpio.rs` | Port latches/directions, mux subset, three independent serial selects, physical buttons, timer drive routing. | Electrical contention/open-drain behavior and some alternate functions remain incomplete. Timer W FTIOA/B/C/D/FTCI are routed through actual package pins. All four pull-up registers now have write/read and electrical-level regression tests; PCR readback remains a latch witness. No invented BMA interrupt wire. |
| `mcu/timer_b1.rs` | Analytical interval/reload counting, distinct load state, overflow IRQ, gated clocks. | Some running reconfiguration combinations stop rather than infer undocumented effects. |
| `mcu/timer_w.rs` | 16-bit compare-on-leaving, overflow, clear-on-A cycle, PWM, paired buffers, input capture, external FTCI, documented write/capture/buffer conflicts, stopped-counter capture and IRQ. | Input pipelines are reference-edge abstractions, not characterized sub-state propagation. Clock-mux glitch behavior is incomplete. TCNT/GR byte accesses are rejected as prohibited by the manual. Buzzer edges still schedule individually; no compressed waveform law yet. |
| `mcu/rtc.rs` | BCD fields, divider/counter state, periodic IRQs, separate busy/update phase, cold versus MCU reset distinction. | Busy duration/placement and intermediate carry behavior are simplified witnesses. The complete non-atomic update sequence and malformed BCD behavior are not established. |
| `mcu/watchdog.rs` | Qualified write handling, counter progression, IRQ/reset cause and warm reset. | Full protected-write qualifications, all address/width nuances and boundary races require verification. |
| `mcu/adc.rs` | Separate sample aperture and conversion result, word result access, channel selection, completion and gating. | Aperture timing is provisional; battery conversion uses a linear switched-supply/3.3 V reference witness. External triggers and active channel/clock changes stop. Full reset retention is not complete. |
| `mcu/ssu.rs` | Holding/shift/receive state, master TX/full-duplex/receive-only, scheduled half-edges, overrun and single-stop sequencing, live SSMR continuation, qualified status clears and sequencer-reset retention. Four independent guest diagnostics cover the receive and register rules. | Slave and bidirectional pin routing remain incomplete. Exact active clock-mux glitches and SOL write-protection transitions need completion. |
| `mcu/sci.rs` | Asynchronous TX/RX, holding and shift state, UART framing, parity, receive status and IrDA pulses. Independent TX/RX fixtures pass. | Synchronous/external-clock and multiprocessor modes are unsupported. Full two-stop-bit receive/error corner cases, mid-frame changes and actual optical transceiver response are incomplete. HGSS peer interoperability has not been tested. |
| `devices/m95512.rs` | Bit-level SPI commands, sequential reads, 128-byte page wrap, WEL/WIP, block protection, status writes, delayed commits. | Five-millisecond programming time is a nominal witness. Board HOLD/WP routes are not invented. Power loss during programming is rejected; no partial-cell model. |
| `devices/bma150.rs` | Physical micro-g input, quantization, bounded filter history, register/shadow state, protected window, working/nonvolatile image, four-wire SPI, basic data-ready/any-motion state. | Exact filter window mapping, rounding, calibration effects, staggered axis publication, interrupt algorithms and physical parameters are not certified. See details below. |
| `devices/nt7508.rs` | 4 KiB RAM plus icons, serial parser, persistent pending parameters across deselection, command/control state, addressing/bitplanes, logical 96x64 rendering. | Analog drive/FRC/PWM/scan timing and several retained analog control effects are not simulated. Panel COM mapping and terminal-column behavior are witnesses. |
| MCU flash | Fixed image reads, execution and mutation-aware design boundaries. | Programming/erase/verify are unsupported. Flash control reads returning reset values and accepted zero writes are scaffolding, not full flash logic. |
| Comparators | Both channels; correct P30 VCref route, ladder/external references, hysteresis, read-armed IRQ baseline, qualified flag clear, vector 36, module/reset behavior; guest wake/replay test and pin stimuli. | Response uses a 15 µs inertial witness (the manual specifies a maximum, not an exact delay). Non-hysteresis VIH follows Table 18.2/Fig.18.2 despite conflicting CRS prose. No characterized analog noise/offset. Digital read suppression on comparator-selected PB pins is a witness; ADC-selected pin suppression is documented. |
| AEC | Independent 8-bit and cascaded 16-bit counters, external edge selection, analytical internal counts, gated clock-return edges, PWM gating/output, read-qualified overflow flags and separate vectors 18/32. Real P10/P11/P12 pin routing and digital fixture inputs; inactive work elision with tested rejoin to shared clock phases. | Reference-prescaler polarity, coincident gate/clock ordering, module-stop details and IRQ synchronizer aperture need physical characterization. Active PWM reconfiguration and undefined ECPWDR reads are rejected. |
| IIC2 | Address region identified; accesses diagnose unsupported behavior. | No functional owner implemented yet. |
| Register-access contract | 95 independently transcribed width/cycle expectations; CPU-boundary tests distinguish two-state RTC/timers/ADC/SPCR/IrCR from three-state SSU/SCI. | Does not certify all instruction microtiming or invalid-width physical outcomes. |
| Frontend/persistence | No-clobber CLI, CSV, raw image exports/import, JSON reports, PGM, ideal-drive WAV tool, typed in-memory snapshot. | No GUI/live-link frontend, no portable snapshot encoding, no atomic multi-file save container. Host export is separate create-new files; report written last. |

## Specific witnesses that must not become invisible assumptions

### Protected BMA150 register 0x1E

Retail initialization reaches this address while protected access is enabled.
The owner models gating and low-level read/write retention. It does not claim
the internal physical meaning of every field at this address is understood.
The test named `protected_window_and_identity` tests the implemented access
contract, not a measured hidden calibration circuit.

### MCU address 0xF088

The supplied firmware writes `3` during infrared initialization. The target
header/reference does not establish a vendor-named register contract for it.
The starter retains bits 0..1 with readback, rejects other set bits, and does not
invent a measured electrical effect. Resolve this against target-specific
silicon/board evidence before extending accuracy claims.

### Sensor front end

Default input is stationary +1 g on Z. Conversion clamps to signed ten-bit
range; nominal range scaling applies. Filter window is currently
`1 << (6 - min(bandwidth, 6))`, with integer truncation and an unfiltered startup
until sufficient history exists. All three axes publish together at nominal
3 kHz. Temperature is a canonical 20 C value. These are explicit model choices,
not verified filter transfer/quantization/axis-skew facts. Sleep stabilization,
image reload and sensor EEPROM timing have separate nominal appointments.

Basic data-ready and any-motion state are present, but their physical board IRQ
connection is not asserted without evidence. Low-g/high-g/alert, autonomous
wake-pause, self-test and three-wire operation stop explicitly. I2C, complete
calibration effects, analog filtering and noise are not implemented.

### LCD viewport

The COM-start-to-visible-row mapping uses the canonical panel's inferred
32-line bonding offset. With the supplied initialization script it produces the
observed home/menu output. It must be validated independently for other COM,
scan and duty configurations. Logical shade rendering is not electrical LCD
scan emulation. Several voltage/oscillator/grayscale settings are stored and
consume parameters, but do not yet modify a calibrated physical display model.

### Power

`Input::ResetPin` is an MCU reset assertion/release; `Machine::power_off/on`
acts on the whole product. Input supply voltage affects the modeled battery
measurement but does not automatically reproduce brownout/reset thresholds.
Cold RAM/CPU unspecified values are initialized deterministically. NMI defaults high as the user-mode strap. A low NMI at reset release/power-on rejects unimplemented bootstrap modes rather than silently executing user firmware. The selected NMI edge is latched independently of IEN/IRR; short-pulse synchronizer behavior is not characterized. Power loss
while either external nonvolatile owner is programming is a rejected lifecycle
request. That is a limitation, not a claim that real hardware refuses power loss.

## Performance boundary

The default engine has no ordinary-run allocations and no second executor.
Inactive AEC clock/pin work is now elided without replaying gated time; see the scoped paired measurement in REVISION-0.2. It still decodes on execution, schedules individual serial/buzzer edges,
and samples the BMA filter at 3 kHz. It does not yet implement all analytical
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
inputs use the existing explicit linear conversion witness. All these inputs
are serialized in in-memory snapshots and validated before a batch mutates the
machine. CMOS contention, clamps and loading from forced voltages are not modeled.
