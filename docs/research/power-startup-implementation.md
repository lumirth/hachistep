# Physical reset and startup implementation contract

Scope: the current starter's `Machine`, MCU control/clocks, and three modeled
external chips. This refines [power-and-reset.md](power-and-reset.md). Keep a
small rail/reset owner, existing clock obligations, and device-owned retention;
do not reconstruct every device whenever the rail changes. The numerical board
realization below is a concrete recommendation, not a claim of measured values.

## Primary constraints

- RES threshold detection starts an **eight-φ release counter**. This is not
  an eight-cycle assertion filter. The separate guaranteed RES-low width is
  20 φ states in active/sleep, or oscillator stabilization plus 20 states at
  cold/stopped-clock reset. [§19, pp.369–370; Table 21.3, p.401][h8-reset]
- The documented RES ratio is 0.8 VCC typical, 0.7–0.9 bounds; its pull-up is
  100 kΩ typical, 60 kΩ minimum. Cold retrigger requires VCC below 100 mV
  **and** removal of RES charge. [Table 21.10, p.408; §19.2.1][h8-por]
- Main-crystal stabilization is 300/800 µs typical/maximum at 2.7–3.6 V,
  600/1000 µs at 2.2–3.6 V; the remaining range has a 50 ms maximum.
  ROSC cold startup is 15/25 µs. Watch-crystal stabilization is at most 2 s
  at 2.2–3.6 V, and 4 s typical below that range. [Table 21.3, p.400][h8-clocks]
- RAM retention is guaranteed from 1.5 V. RES/WDT do not clear RTC time
  registers; cold RTC values are unspecified. No separate brownout-reset
  source is listed. [Table 21.2, p.397; §§11.4.1, 11.6.2; Table 3.2][h8-ram]

The [38606 addition][h8-add] changes memory capacity/addressing, not these
contracts. The base manual's printed page number plus 34 gives its PDF page.

The [original board investigation][board] leaves VCI/RES2B connectivity
unidentified. Matching [HardwareSetup][pw-setup] establishes chip selects,
not independent device power gates. Use the existing common battery rail;
keep host MCU reset independent of LCD RESETB. `pw` calls `AccelInit` directly
after hardware setup and later repeats it; it deliberately executes 15,000
watchdog/delay pairs before setting the RTC. These support separate startup
domains, but do not measure a reset capacitor or oscillator startup time.
[Startup][pw-startup], [sensor initialization][pw-accel], [RTC restoration][pw-rtc]

## Recommended small state

Keep only information that determines future events:

```text
RES: capacitor voltage + last update, external low clamp,
     internal assertion latch, optional remaining 8-φ obligation
Sources: main/watch/ROSC cold-ready deadlines, alongside their existing phases
Retention: accumulated undervoltage exposure + last update + lost flag
Devices: their existing powered/selected/parser/programming/startup state
```

Derive availability from rail voltage and readiness instead of introducing a
parallel operating-mode hierarchy. `power_off` supplies an effective 0 V;
timestamped supply inputs specify the rail itself, including any host-supplied
capacitor hold-up trajectory. Save the state above. An unpowered machine must
still advance physical elapsed time: RC/retention can settle lazily, but their
time must not disappear because `next_devices` is absent.

### RES and reset causes

Choose **100 nF** for the unresolved board RES capacitance, with the published
typical 100 kΩ pull-up and 0.8 ratio. Thus τ = 10 ms. This common component
value gives ample nominal main-crystal and sensor startup time. It is an
inferred board realization, not a photographed component identification.
Do not invent a discharge diode without continuity evidence; initially use
the same resistance for charging and discharge into the supplied rail.

For a constant rail segment, settle the one capacitor analytically:

```text
Vres(t + dt) = Vrail + (Vres(t) - Vrail) * exp(-dt / τ)
```

Use deterministic fixed-point evaluation outside the CPU hot loop. Store
charge through dips; do not snap it to zero on every voltage change. The
manual's C = t/R sizing expression is approximate; the ideal RC crossing
at 0.8 VCC is **τ ln(5)**, not τ. A discharged capacitor at 3 V reaches
2.4 V after 16.094379124 ms under this selected realization.

Interpret the existing `ResetPin(false)` as a low clamp that discharges RES;
`true` removes that clamp and lets the board pull-up charge it. Document this
explicitly if adopted: deasserting the host clamp is not the same instant as
the physical voltage crossing its threshold. A hardware fixture that drives
RES high can bypass the RC ramp without bypassing the eight-edge counter.

At a valid MCU supply, a low RES comparator output asserts reset and clears
the release counter. When the comparator becomes high, count the next eight
actual **φ** edges, excluding an edge already consumed at that boundary.
Reassertion discards partial qualification. An unavailable clock earns no
edges. Upon completion, admit the existing vector-read / two-state internal
wait / first-prefetch CPU sequence; do not replace it with a PC assignment.
Sample reset straps at actual internal reset exit, not host-clamp release.
An already-running oscillator is not rephased by reset.

The minimum assertion widths above are guarantees for software/hardware
designers. A shorter input is not a host API error. The compact selected
realization still accepts its asynchronous assertion; it does not invent a
20-cycle debounce filter or claim all physical short pulses are accepted.

Combine reset causes. A WDT reset retains its documented 512-ROSC hold.
If RES was already qualified, **do not append another eight φ clocks to
WDT release**. Conversely, WDT expiry cannot release an outstanding RES hold.
The RES counter belongs to its voltage detector, not to every call that
initializes MCU registers. [§12.3.1, p.208; §19 Fig.19.1][h8-wdt]

### Cold sources and warm transitions

Select these cold delays when a demanded source first obtains valid MCU
supply; a source whose physical supply was lost starts a fresh phase:

| Source | Selected cold availability delay | Basis |
| --- | --- | --- |
| Main crystal | 300 µs at ≥2700 mV; 600 µs at 2200–2699 mV; 50 ms at 1800–2199 mV | Typical values where given; published maximum for the lowest band. |
| ROSC | 15 µs | Published cold typical. |
| Watch crystal | 2 s at ≥2200 mV; 4 s at 1800–2199 mV | Published upper bound / low-voltage typical; no nominal value published for the upper band. |

This is a deliberately **lumped cold-start waveform**: no qualified edges
before its deadline, then the ordinary configured oscillator frequency.
The manual specifies stabilization, not the first digital edge, so equating
those instants is the chosen approximation. Do not describe these defaults
as measured oscillator latency. Capture the band on startup; an ordinary
within-range rail change does not restart its timer. Loss of functional
supply cancels the start, and recovery begins it again.

Keep cold startup distinct from the existing warm main-oscillator wake
sequence. Section 4.5.3, pp.74–75 separates initial oscillation onset from
the STS-counted wait. **Do not add the published full stabilization interval
after an STS wait, or make STS progress while its source is unavailable.**
For the initial implementation, retain the existing nominal-edge waveform
and STS counting for warm main restarts; the new delays above apply after
rail loss. This leaves the unmeasured first-edge part of warm resonator
startup as a specific remaining waveform approximation, rather than adding
a second protective wait that silently repairs too-short guest settings.
[§4.5.3 and Fig.4.12][h8-warm]

A still-powered watch crystal retains phase through MCU reset and standby
when SUBSTP permits it. Explicitly stopping that crystal and restarting it
does need the selected watch startup delay. ROSC starts when newly demanded
and no longer has the cold delay if it was already running. Reset makes ROSC
demanded because of the WDT reset configuration. Watch readiness must not
hold an otherwise ready main CPU, EEPROM, LCD, or BMA150. [§5.5, p.96][h8-rosc]

### Retained bits through a short collapse

Immediately filling RAM on `power_on` is unsupported. Preserve MCU RAM and
retained RTC state until a separate, finite loss exposure has elapsed.
Use this simple canonical realization:

```text
D += max(1500 - Vrail_mV, 0) * dt_ms
loss when D >= 15000 mV·ms
at Vrail >= 1500 mV, replenish D to zero
```

This chooses **10 ms at 0 V**, 30 ms at 1.0 V, and 150 ms at 1.4 V before
the modeled volatile state is lost. Only the 1.5 V guaranteed retention
boundary is a manufacturer fact; the finite exposure law and 10 ms constant
are an explicit unmeasured realization of residual cell charge. Keep this
constant centralized for later calibration. A zero-duration power toggle
must not erase RAM. A deep, sustained collapse must not preserve it forever.

Once lost, use the current deterministic cold RAM pattern and cold RTC
realization; this is not an all-zero guarantee of the chip. On recovery
before loss, apply only reset effects actually caused by RES/WDT. A short
dip can stop CPU execution, preserve RAM, and either cause or avoid reset
depending on remaining RES charge. For below-rated execution, choose frozen
retained logic and no clocked side effects until supply recovers; that is a
bounded realization of an unrated condition, not a promise of silicon
brownout survival. Reuse this one exposure for LCD RAM retention initially,
explicitly as an inference; LCD reset commands must still retain its RAM.

## External-chip availability

Apply rail changes to each owner. These lower limits are conservative
functional gates, not newly claimed POR thresholds or hysteresis pairs.

| Owner | Functional gate and recovery contract |
| --- | --- |
| MCU | Below 1.8 V, halt clocked activity and active pin drive; keep retained state and remaining obligations. At valid supply, restore demand through source readiness and RES qualification. |
| M95512-R | Require ≥1.8 V for serial operation and programming. Interrupt an in-progress write at loss of valid supply using its existing partial-cell model; discard partial serial commands and require CS high before a fresh falling edge. Stable valid supply has no additional specified fixed startup timer. Full rail loss clears WEL/WIP and preserves nonvolatile status/data. |
| BMA150 | Require VDD ≥2.4 V and VDDIO ≥1.62 V; on this common rail use 2.4 V for whole-chip functional availability. VDDIO's lower minimum alone does not establish that the ASIC core can answer SPI with invalid VDD. Full rail loss reloads EEPROM image and starts a 3 ms cold interval, with no valid sample/serial response before readiness in the selected model. A nonzero, retained undervoltage dip keeps configuration and uses the existing 1 ms reacquisition interval on recovery; no fabricated new samples during absence. |
| NT7508 | Gate digital interface/oscillator below 1.65 V; gate analog LCD drive below 2.4 V. Analog-only loss removes electrode drive but does not clear RAM, addressing, or logical scan phase. Full power restoration uses the controller's hardware-reset control state, retaining RAM if the exposure above has not expired. Model the board's required cold RESETB hold as 20 µs plus 1 µs reset completion, independent of the host MCU reset input. This hold is a chosen board realization, not an internal NT7508 POR timer or established net connection. |

Sources: ST [§5.1, p.8; Table 10, p.25][st]; Bosch [Table 1, pp.5–6;
§§3.3.6–3.3.7, p.21; §7.2, p.46][bosch]; Novatek [pp.29, 52, 56, 62][nt].
At zero rail all active drive, conversion, programming, scan drive, and RTC
progress cease. Static sensor input and host button positions remain physical
conditions. Device soft reset, sleep, module gating, and MCU reset are not
substitutes for these supply rules.

At a supply-loss boundary, settle previous valid progress, disable the affected
device, and then resolve changes from unavailable MCU outputs. Do not turn a
depowered CS transition into a newly accepted EEPROM write. At valid-voltage
MCU reset, the opposite distinction matters: real GPIO/CS changes still reach
powered devices and an already accepted EEPROM operation continues.

## Independently derived conformance scenarios

Keep hardware requirements separate from tests of chosen numerical defaults.
Use a 4 MHz main-clock fixture for the following literal times; frequency is
within the target's rating. These values come from circuit equations and edge
counting, not by running the implementation and recording its output.

| Kind | Stimulus | Expected witness |
| --- | --- | --- |
| Documented counter | A fixture raises the actual RES voltage above threshold at 0.125 µs; existing φ edges are at 0.25, 0.50, … µs. | Reset remains held through 1.75 µs; releases on the eighth edge at 2.00 µs. Vector read starts only then; its two-state transfer completes at 2.50 µs. |
| Documented restart | In that counter case, make RES low again after six qualifying edges, then raise it at 3.125 µs. | Old six edges are discarded; release is at 5.00 µs, after eight new edges. |
| Selected cold realization | At t=0 supply 3 V to a discharged 100 nF RES capacitor; cold main becomes available at 300 µs, phase starts there. | Threshold is at 16094.379124 µs; first qualifying edge at 16094.50 µs, eighth at 16096.25 µs. Vector transfer completes at 16096.75 µs. No CPU bus effects at an exclusive horizon equal to that completion time. |
| Selected source realization | Same cold start. | No ROSC edges before 15 µs, main edges before 300 µs, or watch edges before 2 s. Each first edge occurs one configured period after readiness. Main CPU can run before the watch crystal; no RTC watch count is backfilled at 2 s. |
| Selected short-collapse realization | Fully charged RES at 3 V; hold rail at 0 V for 1 ms, then restore 3 V. | Vres = 2714.512254 mV, above 2400 mV; no new MCU reset merely from restoration. RAM/RTC bits survive. CPU/RTC had no edges while the rail was absent; stopped physical oscillators restart through readiness. External cold-reset effects remain device-owned. |
| Selected reset-with-retention realization | Same, but collapse lasts 5 ms. | Vres = 1819.591979 mV; RAM still survives, but RES asserts on restoration. Its threshold is reached 6766.857829 µs later; with cold main epoch at 300 µs, internal reset releases at 6768.75 µs. |
| Selected retention realization | Write a RAM marker, then apply 0 V for 9 ms versus 10 ms; separately 1.0 V for 29 ms versus 30 ms. | First member of each pair retains it; second reaches the modeled loss budget. A RES-only reset at 3 V retains it indefinitely. |
| Independent domains | MCU reset during an already accepted EEPROM write, rail remains 3 V. | Partial CPU work is aborted; EEPROM reaches its existing completion time; sensor/LCD do not receive synthetic soft resets. WDT-only hold remains exactly 512 ROSC cycles, without an added RES counter. |
| Functional gates | Hold rail at 2.0 V, then 1.7 V, then restore 3 V. | At 2.0 V MCU/EEPROM and LCD digital logic can operate; BMA and LCD analog drive cannot. At 1.7 V MCU/EEPROM also stop. Neither crossing is itself a universal POR or RAM clear. |
| Causal snapshot | Snapshot during RC charging, after four release edges, during watch startup, and during retained rail absence. | Restore keeps the original charge, consumed edges, source deadlines, and retention dose. Subsequent effects and timestamps match; no timer restarts at restoration. |

## Concrete current gaps and correction order

Checkpoint: the external RES release counter now uses the existing `ClockWait`
with eight reference-clock edges. Reassertion, exact exclusive endpoints,
snapshot restoration and overlap with the independent WDT hold are tested.
The remaining capacitor, supply readiness and retention work below is pending.

1. [`Machine` construction/power/reset](../../crates/hs-core/src/machine.rs)
   admits execution for any nonzero rail, and pin release immediately removes
   `hold_reset`. Add RES qualification and cold source availability before
   accepting CPU work. Keep WDT and RES cause latches distinct.
2. [`Mcu::power_on`](../../crates/hs-core/src/mcu/mod.rs) always rebuilds clocks,
   clears RAM, and reconstructs RTC. Split rail recovery, actual reset, and
   volatile loss so short dips and powered reset retain the correct domains.
3. [`Clocks`](../../crates/hs-core/src/mcu/clocks.rs) creates/restarts sources
   immediately. Add physical readiness to source demand, without changing
   shared prescaler ownership or advancing clock obligations while absent.
4. [`Machine::power_off/power_on`](../../crates/hs-core/src/machine.rs) globally
   suspends scheduling and recreates the LCD; nonzero undervoltage only changes
   conditions. Add the per-owner availability rules and preserve LCD RAM on
   short interruption. The BMA already has a sample cold-start delay; its
   serial availability and power return must use the same cold state.

No production code, fixtures, tests, builds, or commits were changed for this
investigation. The primary timing/counter rules can be implemented directly.
Capacitance, absence of an extra RES discharge path, first-edge startup
waveforms, and retention exposure are the specifically identified physical
constants/approximations to replace when board measurements become available.

[h8-reset]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=403
[h8-por]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=442
[h8-clocks]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=434
[h8-ram]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=431
[h8-wdt]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=242
[h8-warm]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=108
[h8-rosc]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=130
[h8-add]: https://www.renesas.com/en/document/tcu/addition-h838606-group
[board]: https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/doc/Board.md
[st]: https://www.st.com/resource/en/datasheet/m95512-df.pdf
[bosch]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf
[nt]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf
[pw-setup]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_ssu_init.c#L4-L18
[pw-startup]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L193-L245
[pw-accel]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_accel_bma150.c#L68-L95
[pw-rtc]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_storage.c#L147-L159
