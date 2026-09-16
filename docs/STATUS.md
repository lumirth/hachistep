# Implementation status and fidelity boundary

The starter is runnable and regression-tested. It is **not** a complete
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
| `cpu/decode.rs` | Incremental decoding of H8/300H normal-mode scalar, memory, branch, bit, control, multiply/divide, exception and block-copy forms; all 65,536 first words classify without panic. | Classification totality is not complete encoding certification. Invalid/reserved selector handling needs a full independently reviewed encoding corpus. |
| `cpu/alu.rs` | Explicit 8/16/32-bit arithmetic, register aliases, CCR updates; exhaustive byte add/sub input cases. | Wider flag cases, unusual DAA/DAS incoming flags and undefined divide cases need broader independent evidence. Divide-by-zero/quotient overflow stop explicitly. |
| `cpu/mod.rs` | One `next`/`complete` continuation engine; partial loads/stores, calls/returns, PC-before-CCR exception stack writes, TRAPA/RTE, EEPMOV, RAM execution. | Complete instruction microtiming, discarded/prefetched accesses, interrupt admission/deferral, EEPMOV NMI behavior and unspecified CCR-word bits remain incomplete. No prefetch queue is claimed. Some internal effects occur at provisional phase boundaries. |
| `machine.rs` | Exclusive horizons, physical timelines, cached next-device boundary, callbacks, fault latching, snapshots, reset/power API. | Same-time ordering is currently devices, then input batch, then CPU; it is a deterministic witness, not each register's silicon conflict rule. In-flight CPU waits use absolute appointments rather than complete retained edge obligations across arbitrary clock changes. |
| `mcu/clocks.rs` | Shared divider ordinals, main/watch/on-chip sources; integer source frequencies. | Nominal 3,686,400 / 32,768 / 1,310,720 Hz are model choices, not measurements. Switching the system source retains edge ordinal but rephases its fractional timing. No arbitrary rational frequency through public `Conditions` yet. |
| `mcu/control.rs` | Main/subactive/sleep/subsleep/watch/standby paths used by retail, module gates, external IRQs and wake transitions. | Some invalid or external-source configurations stop. All retention matrices, oscillator transients and simultaneous admission races are not complete. |
| `mcu/gpio.rs` | Port latches/directions, mux subset, three independent serial selects, physical buttons, timer drive routing. | Full pull/electrical contention/open-drain behavior and alternate functions are incomplete. Some register readback is a latch witness. No invented BMA interrupt wire. |
| `mcu/timer_b1.rs` | Analytical interval/reload counting, distinct load state, overflow IRQ, gated clocks. | Some running reconfiguration combinations stop rather than infer undocumented effects. |
| `mcu/timer_w.rs` | 16-bit compare-on-leaving, overflow, clear-on-A cycle, PWM output latches, flags and IRQ. | Capture, buffering, external clock and byte access modes are unsupported. Buzzer edges currently schedule individually; no compressed waveform law yet. |
| `mcu/rtc.rs` | BCD fields, divider/counter state, periodic IRQs, separate busy/update phase, cold versus MCU reset distinction. | Busy duration/placement and intermediate carry behavior are simplified witnesses. The complete non-atomic update sequence and malformed BCD behavior are not established. |
| `mcu/watchdog.rs` | Qualified write handling, counter progression, IRQ/reset cause and warm reset. | Full protected-write qualifications, all address/width nuances and boundary races require verification. |
| `mcu/adc.rs` | Separate sample aperture and conversion result, word result access, channel selection, completion and gating. | Aperture timing is provisional; battery conversion uses a linear switched-supply/3.3 V reference witness. External triggers and active channel/clock changes stop. Full reset retention is not complete. |
| `mcu/ssu.rs` | Separate holding/shift/receive state, scheduled half-edges, complete selected transfers, status and gating. | Supported master four-wire path only. Slave, bidirectional, RX-only and active format/clock changes stop. Full pin/mode conflicts are not characterized. |
| `mcu/sci.rs` | Asynchronous TX/RX, holding and shift state, UART framing, parity, receive status and IrDA pulses. Independent TX/RX fixtures pass. | Synchronous/external-clock and multiprocessor modes are unsupported. Full two-stop-bit receive/error corner cases, mid-frame changes and actual optical transceiver response are incomplete. HGSS peer interoperability has not been tested. |
| `devices/m95512.rs` | Bit-level SPI commands, sequential reads, 128-byte page wrap, WEL/WIP, block protection, status writes, delayed commits. | Five-millisecond programming time is a nominal witness. Board HOLD/WP routes are not invented. Power loss during programming is rejected; no partial-cell model. |
| `devices/bma150.rs` | Physical micro-g input, quantization, bounded filter history, register/shadow state, protected window, working/nonvolatile image, four-wire SPI, basic data-ready/any-motion state. | Exact filter window mapping, rounding, calibration effects, staggered axis publication, interrupt algorithms and physical parameters are not certified. See details below. |
| `devices/nt7508.rs` | 4 KiB RAM plus icons, serial parser, persistent pending parameters across deselection, command/control state, addressing/bitplanes, logical 96x64 rendering. | Analog drive/FRC/PWM/scan timing and several retained analog control effects are not simulated. Panel COM mapping and terminal-column behavior are witnesses. |
| MCU flash | Fixed image reads, execution and mutation-aware design boundaries. | Programming/erase/verify are unsupported. Flash control reads returning reset values and accepted zero writes are scaffolding, not full flash logic. |
| IIC2, AEC, comparators | Address regions identified; accesses diagnose unsupported behavior. | No functional owner implemented yet. These remain substantial custom-firmware gaps. |
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
Cold RAM/CPU unspecified values are initialized deterministically. Power loss
while either external nonvolatile owner is programming is a rejected lifecycle
request. That is a limitation, not a claim that real hardware refuses power loss.

## Performance boundary

The default engine has no ordinary-run allocations and no second executor.
However it still decodes on execution, schedules individual serial/buzzer edges,
and samples the BMA filter at 3 kHz. It does not yet implement all analytical
edge-run and signal-law optimizations in the design. No fastest-emulator or
mobile energy-efficiency claim is made. See actual samples in `evidence/`.

## Consequence for custom firmware

The implemented mechanisms can be exercised by custom images, including RAM
execution and independently assembled diagnostic programs. A complete clean run
is useful evidence but cannot establish that the firmware transfers correctly to
hardware outside the listed coverage. Consult this matrix, use focused fixtures,
and do not reinterpret stored-only witnesses as full peripheral emulation.
