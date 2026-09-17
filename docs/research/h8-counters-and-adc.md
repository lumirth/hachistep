# H8/38606F counters, RTC and ADC

Research of the current starter, 2026-09-16. The target addition leaves these
peripherals unchanged, so the applicable baseline is **REJ09B0152-0300 rev. 3.00**.
Page references are printed manual pages; PDF pages are 34 higher.
([Target applicability][target], [manual][manual])

## Implement documented operations; infer off-sequence operations locally

The manual distinguishes a protected write, a reserved readable/writable bit,
and a programming sequence whose result is not guaranteed. These are different
hardware cases. A guest performing the latter still executes on a real chip.
Replace the remaining guest-triggered `Unsupported` branches with ordinary
register/state transitions and small, deterministic local inferences. Keep those
inferences next to the affected mechanism, not as a second execution mode.

For every live reconfiguration: settle old-source activity through the bus
access, perform the write, evaluate affected gates/muxes, and project the next
event from retained progress. Do not reset a counter/divider merely because
software changed its configuration.

## Timer B1

`F0D0 TMB1` resets to `38`: bit 7 selects reload, bit 6 enables counting, bits
2–0 select `φ/8192, /2048, /256, /64, /16, /4, φW/1024, φW/256`.
`F0D1` reads TCB1 and writes TLB1. A stopped TLB1 write updates **both** load and
counter, including interval mode. Overflow sets IRR2.IRRTB1, and reload occurs
on that overflow; the first traversal is `256-count`, subsequent traversals
are `256-load` in reload mode. The existing arithmetic already preserves this.
([Manual §§9.2–9.4, pp. 146–150][timer])

The supported setting procedure explicitly stops the counter before changing
mode, source or load. Running changes are not documented as ignored or as CPU
faults. **Recommended inference:** accept the register write; a live TLB1 write
loads both latches, while mode/source changes retain the counter. Continue from
the newly selected shared prescaler phase. A mux-induced edge can be evaluated
through the existing source-level convention; do not automatically grant a new
full period. This replaces both running-write errors without changing normal
firmware behavior.

Both sources run in active/sleep. Watch-source counting also runs in watch,
subactive/subsleep, and their wake stabilization. All B1 counting halts in
standby and standby wake stabilization; counters are retained. System-source
counting stops outside active/sleep. Module standby retains state; reset clears
mode/count/load. ([Manual table 9.1, p. 151; §20.3][timer-power])

## RTC: retained state, busy interval and raw counters

**Reset correction:** RTCFLG, time data and RTCCR1/2 survive RES/watchdog reset,
but **RTCCSR is initialized to `08`**. Section 20.3 explicitly lists that
exception. Current `Mcu::reset` preserves the whole `Rtc`, including RTCCSR.
Software `RTCCR1.RST=1` instead resets RTC registers/control circuits except
RTCCSR and RST itself; software must clear RST. Cold RTC contents are not
specified; zero remains a reasonable deterministic construction value, distinct
from these reset rules. ([Manual pp. 191, 196, 199, 380][rtc-reset])

The stable read contract is specific: BSY becomes one, approximately 62.5 ms
later data registers update and BSY clears. INT selects periodic interrupts
during busy or immediately afterward. The current 512 ticks of `φW/4` busy
duration and separate commit are a sound nominal representation. Its placement
at ticks 7680–8192 is an inference; neither the manual nor the same-target RTC
application note fixes first-BSY phase after RUN. Retain this local phase
choice; there is no basis to invent a staggered per-register update sequence.
([Manual §11.4.3][rtc-busy], [RTC Operation §3.2–3.3, pp. 8–9][rtc-app])

Keep quarter/half-second divider events distinct from calendar update. Set
RTCFLG flags only for enabled RTCCR2 sources; clearing is write-zero, with no
prior-read qualification. Calendar mode uses `φW/4`; the other documented
sources make RSECDR a full 8-bit binary free-running counter. In calendar mode
bit 7 reads BSY, not a raw time-data bit. Stop/gating preserves phase; RTC RST
clears it. Watch calendar counting continues through watch/subactive/subsleep
but halts in standby. ([Manual §§11.3, 11.5; table 5.3][rtc])

RTCCSR also selects TMOW: upper-field values `000/010/100/110` select
`φ/4,/8,/16,/32`, and `xx1` selects φW. Route this through PMR1's existing pin
mux from the actual divider signal; changing clock output must not reset time
counting. ([Manual §11.3.7, p. 193][rtc])

Concrete replacements for the remaining RTC errors:

- **Time write while RUN:** accept the documented writable field bits
  (`7F,7F,3F,07`) without resetting the divider. The prescribed software
  sequence stops and resets before setting time; it does not describe a hardware
  write lock. As a compact race model, latch the prospective update at busy
  entry; writes change visible storage, and an already-started update commits
  its latched result afterward. Stopping freezes that update; RST cancels it.
  These race outcomes are implementation inferences.
- **Malformed BCD:** preserve raw field widths and use digit-counter terminal
  tests. For seconds/minutes, units `9` clears and carries; otherwise increment
  the four-bit units field. On a units carry, tens `5` clears and carries;
  otherwise increment its three-bit field. For hours, `11`/`23` is the selected
  terminal, with ordinary BCD digit carry otherwise. Day `6` clears and carries;
  other values increment modulo eight. Thus invalid digits wrap by their
  physical widths, not by calendar normalization, and never cause a Rust
  overflow or an emulator error. PM toggles only at the 12-hour terminal; day
  advances when PM changes from one to zero; the weekly condition checks the
  resulting day value for zero. This is a small comparator/counter
  inference, consistent with the described digit structure.
- **RTCCSR codes 9–15:** retain the written register and decode bit 3 as watch
  calendar selection. This has stronger evidence than a guessed clock: the
  same-target 2005 application note explicitly lists `1xxx` as RTC operation,
  whereas rev. 3 narrows permitted programming to `1000`. Treat the alias as an
  inference, not a reason to advertise those settings as supported hardware
  programming practice. ([ADC application note p. 12][adc-app])

## Watchdog

`FFB0–FFB3` are TMWD/TCSRWD1/TCSRWD2/TCWD. Retain the current distinct write
qualifiers: write-inhibit bits read one; TCWE controls counter writes; existing
TCSRWE plus the appropriate zero write-inhibit bit controls WDON/WRST; OVF
requires a prior read of one before a zero write clears it. TCWD writes preload
without restarting its source divider. WRST survives watchdog reset and clears
on RES or its qualified software write. ([Manual §12.2, pp. 203–207][wdt])

Two substantial missing behaviors:

1. **Reset is timed.** Overflow in watchdog mode asserts internal reset for
   **512 ROSC cycles**. Retain reset assertion/release as machine state, including
   ROSC phase, while independent RTC/external devices continue. An immediate
   `reset_mcu` followed by CPU execution omits that interval.
   ([Manual §12.3.1, p. 208][wdt-operation])
2. **Qualified clear has an instruction-address erratum.** In interval mode,
   clearing WT/IT or IEOVF can fail depending on the second-lowest bit of the
   transfer instruction's address. The rev. 3 manual gives absolute-8 MOV.B
   examples: address modulo four equal to two succeeds; modulo four equal to
   zero may fail. TN-H8*-A309B/E confirms interval-mode scope and the two-site
   retry workaround. Preserve instruction PC/addressing provenance at the bus
   write. A deterministic implementation can use the documented failure for
   the modulo-four-zero absolute-8 case; do not generalize it into failure of
   every write or every addressing mode. ([Manual pp. 210–211][wdt-notes],
   [A309B/E, p. 1][wdt-erratum])

WDON overrides WDCKSTP: clearing module-enable while counting does not stop the
watchdog. The unused `gate` field is therefore not, by itself, evidence that
WDON counting should be disabled. **Source availability still matters:** φ
sources halt outside active/sleep; φW sources run in watch/subactive/subsleep
but halt in standby; ROSC can operate in every mode. Current WDT sync/deadline
uses WDON alone while the shared clock objects keep advancing. Add these source
gates without treating WDCKSTP as an override of WDON. Preserve count on halt.
([Manual §12.5.2; table 5.3 notes 4–5; §4.3.4][wdt-power])

Active WT/IT changes are warned to cause count errors; use retained count and
the new overflow action as the nominal inference. Reserved CKS codes 6–7 can
retain their raw bits and disconnect the counter clock until a valid selection
returns; do not map them accidentally to φ/64. This removes the reserved-clock
error without inventing a documented clock source.

## ADC

The current conversion lengths match AMR: CKS `00/01/10/11` means at most
`124φ / 62φ / 31φ / 31φW` states. Channels 4–9 select AN0–AN5; other codes
disconnect the input mux. ADSF zero aborts, one starts; successful completion
commits ADRR bits 15–6, clears ADSF and sets IRRAD together. Changing channel
or conversion speed is prescribed with ADSF cleared. ([Manual §§17.3–17.4][adc])

**Definite reset fix:** ADRR survives MCU reset; AMR and ADSR reset. Current
`Mcu::reset` constructs `Adc::default()` and loses the result. Sleep preserves
conversion; watch/standby/module standby halt and retain it. Subactive/subsleep
permit watch-source conversion, with the documented restriction that CPU φSUB
must equal φW. Do not turn a program violating that restriction into a core
exception; retain the source-domain conversion as the nominal inference.
([Manual table 17.2, p. 354][adc-power])

Suggested compact conversion state is a phase in **31 half ADC-clock steps**,
a held analog sample and a pending result. Each step spans `4φ, 2φ, φ, φW`
for the four CKS values, reproducing all four totals. This makes an active CKS
change consume the remaining converter work on the newly selected source,
rather than restarting 31/62/124 states. Keep the held sample unchanged after
capture. Before capture, a channel write affects the mux normally. For a
disconnected mux, retain the previous sample-capacitor value and complete
normally. These live-reconfiguration outcomes are inferences consistent with
the shared sample-and-hold/SAR structure, and replace both conversion errors.

The current aperture at four *undivided* reference edges is not specified by
the manual. Preserve its fastest-setting placement as an initial inference,
but express it as four of those 31 converter steps so slower settings scale
with the converter clock. Retain readiness separately: §17.7.3 requires
**10φ cycles after leaving module standby** before software starts conversion.
This is an analog-settling requirement, not permission to insert an automatic
CPU wait. For premature starts, continuing conversion with the previously held
sample if acquisition precedes readiness is a usable initial inference.
([Manual pp. 359–360][adc-settling])

Implement external trigger through the physical TEST/ADTRG input: PMRB bit 3
selects ADTRG, AMR.TRGE enables it, IEGR bit 5 selects falling (`0`) or rising
(`1`). Synchronize detection with the operating clock and enter the same ADSF
start path. A trigger while already converting need not queue/restart a
conversion; ignoring it is the natural level-ADSF inference. Do not replace
this with a host command that writes a conversion result. ([Manual §3.4.3,
§8.5.2, §17.4.2, figure 17.2][adc-trigger])

## AEC: active reconfiguration

The underlying counter/gate model already covers much of §13. Preserve that
structure, and replace the refusal branches as follows:

| Current refusal | Concrete transition |
| --- | --- |
| ECPWDR read | Return a deterministic bus value, initially zero; it is write-only with undefined read value, not a CPU fault. |
| AEGSR bit 0, ECCR bit 0, ECCSR bit 5 | Store/read them: the manual explicitly calls these reserved bits **readable/writable**. They have no modeled function. |
| Edge selection `11` | Retain encoding; select neither edge as the local inference. |
| PWCK `111` | Retain encoding and PWM phase; disconnect its clock as the local inference. |
| Active period/duty write | Update the latch and retain the 16-bit PWM count and output phase. Compute the next comparator equality with wrapping distance, not subtraction that assumes count ≤ new threshold. |
| Active PWCK change | Keep PWM count/output; select the new divider's existing phase. |
| Live CUEH/CH2 changes | Preserve H/L counts except explicit CRCH/CRCL reset; update the cascade/independent clock gates. Apply any resulting clock edge through the same counter mechanism. |

The running period/duty/clock transitions above are inferences: software is
told to stop PWM first. Stable waveform facts are exact: low time is
`(ECPWDR+1)` clocks, full period `(ECPWCR+1)` clocks, and duty ≥ period forces
IECPWM low. A live write causing that last condition must also produce the
corresponding gate transition; merely suppressing future deadlines can leave
the current implementation's output incorrectly high.
([Manual §§13.3–13.4.5, pp. 216–227][aec])

For 16-bit operation the manual specifically warns that changing CUEH after
CRCH release can miscount ECH; it also requires CUEH set before or together
with CRCH release. Consequently the cascade should have a retained clock/gate
level, not only a conditional integer carry. The exact illegal-sequence
miscount is a local characterization target. A nominal implementation may
preserve count on reconfiguration unless its modeled gate creates an edge;
do not claim that accepting the write proves that glitch is reproduced.
([Manual §13.6.3, p. 229][aec-notes])

External AEV counting remains available even in standby and wake stabilization;
internal φ counting does not. PWM φW/16 survives watch/subactive/subsleep and
their wake stabilization, but halts in standby; PWM output is high impedance
during standby and its stabilization. Module standby stops the module, while
ordinary power-mode halts retain counts/phase. Counter overflow requests and
IRQAEC/IECPWM edge requests remain distinct; the latter has up to one CPU/subclock
cycle synchronization delay. ([Manual table 13.3 and §13.6.6, pp. 228–230][aec-power])

## Reached firmware behavior

Public `lumirth/pw` commit `6dc7bc09950078fa3fe0dffa4dae34e9549a99da` corroborates
the ordinary paths without supplying results for every off-sequence write:

- [TimerB1Init][pw-timer] stops/configures, loads `F8`, then starts watch/256
  reload counting: eight input clocks, hence 62.5 ms at 32.768 kHz.
- [RTC setup and stable read][pw-rtc] explicitly resets RTC, enables INT-after-busy,
  and takes two complete busy-qualified snapshots.
- [Battery ADC loop][pw-adc] wakes the module, executes five NOPs, selects AN3 and
  CKS `10` or `11`, polls ADSF, gates the module off, then reads retained ADRR.
  Five two-state NOPs match the documented 10φ settling requirement.
- [Watchdog controls][pw-watchdog] perform the qualified unlock sequences;
  [startup][pw-reset] observes WRST and updates an EEPROM diagnostic counter.
  This makes reset-cause preservation consequential to retail execution.

The searched public `src/` contains no AEC register programming; AEC live-write
behavior above is grounded in hardware rather than attributed to retail code.

[manual]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[target]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=2
[timer]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=180
[timer-power]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=185
[rtc]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=221
[rtc-reset]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=414
[rtc-busy]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=231
[rtc-app]: https://www.renesas.com/en/document/apn/h838602r-group-rtc-operation#page=10
[adc-app]: https://www.renesas.com/en/document/apn/h838602r-group-application-note-ad-conversion-using-subclock#page=14
[wdt]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=237
[wdt-operation]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=242
[wdt-notes]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=244
[wdt-erratum]: https://www.renesas.com/en/document/tcu/h838086r-group-h838076r-group-h838602r-group-watchdog-timer-usage-note-0#page=2
[wdt-power]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=120
[adc]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=385
[adc-power]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=388
[adc-settling]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=393
[adc-trigger]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=387
[aec]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=250
[aec-notes]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=263
[aec-power]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=262
[pw-timer]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L35-L44
[pw-rtc]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_rtc.c#L164-L206
[pw-adc]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_battery.c#L55-L87
[pw-watchdog]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/lib_common.c#L582-L598
[pw-reset]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L193-L209
