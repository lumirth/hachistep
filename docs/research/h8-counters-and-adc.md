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

## Watchdog implementation plan, 2026-09-17

Rechecked `mcu/watchdog.rs`, MCU gate/reset routing, CPU access provenance, and
the machine's reset scheduling. The current owner already models the main
write-inhibit rules. The missing work is source availability and divider state,
a real reset interval, and the address-sensitive clear operation. The target
addition does not replace WDT behavior. Use [manual §§12.1–12.5, pp. 201–211][wdt]
and **TN-H8*-A309B/E, rev. 2, 2005-10-04**, rather than its superseded A revision.

### Counter source, gates, and retained phase

`FFB0–FFB3` are TMWD/TCSRWD1/TCSRWD2/TCWD, byte-wide registers with two-state
bus accesses (table 20.1, p. 374). Reset values are `F0/AE/57/00`; WDON starts
enabled. TCWD is an eight-bit up-counter, and the transition `FF→00`, not
reaching FF, causes overflow. TMWD selects:

| CKS | Source | Full 256-count interval with current canonical frequencies |
| --- | --- | --- |
| 0–3 | ROSC/2048, documented aliases | 400 ms at ROSC=1,310,720 Hz |
| 4 | φW/16 | 125 ms at 32,768 Hz |
| 5 | φW/256 | 2 s at 32,768 Hz |
| 6–7 | Reserved | Retain encoding; disconnect counter clock as the selected rule. |
| 8–15 | φ/64, /128, /256, /512, /1024, /2048, /4096, /8192 | 4.444444 ms through 568.888889 ms at φ=3,686,400 Hz |

The ROSC frequency is a unit parameter; the current default reproduces the
manual's 0.4-s typical overflow period, not a precision-oscillator guarantee
([table 21.9, p. 407][wdt-electrical]). A TCWD preload `x` leaves exactly
`256−x` selected input edges, while the first edge retains the source's phase.

WDON gates counting. Effective module operation is `WDON || WDCKSTP`: clearing
WDCKSTP while WDON=1 records the bit but cannot stop TCWD. Once WDON becomes
zero, WDCKSTP=0 takes effect. Module standby retains registers; it does not
reset or clear pending OVF. The shared source has separate availability:

- ROSC-selected WDT works in active, sleep, watch, subactive, subsleep, and
  standby. ROSC operates during reset as well. It stops only when none of its
  WDT/system/subclock consumers needs it; the reset hold is itself a consumer.
- φ-source WDT works in active/sleep, including the selected medium-speed
  divider. It halts in watch, subactive, subsleep, standby, and their main-clock
  stabilization. Do not accidentally substitute CPU φSUB for φ in those modes.
- φW-source WDT continues in watch/subactive/subsleep and watch wake
  stabilization. It halts in standby and its stabilization. Apply the physical
  watch-source stop/mux rules too, including SUBSEL's ROSC/32 source.

These gates follow [table 5.3 notes 4–5][wdt-power], §12.5.2, p. 210, and
[§5.5, p. 96][wdt-rosc]. They preserve TCWD on a halt. A resumed counter joins
the next real selected edge; it must not count the elapsed stopped interval.

Preserving the oscillator is not the same as preserving every divider.
**Prescaler S resets to zero on MCU reset and on entering watch, subactive,
subsleep, or standby. Prescaler W resets on MCU reset, holds in standby, and
continues through watch/subactive/subsleep. Both start counting on reset
release.** These are explicit [§4.4, p. 71][wdt-prescalers] rules. System/watch
WDT taps must use those shared dividers, not `global_source_ordinal/divisor`
across every transition. For the undocumented private ROSC/2048 divider, choose
the corresponding small circuit model: retain its phase across WDON changes
and ordinary source selection, hold it when its module/source stops, and clear
it with MCU reset. Treat that private-divider reset placement as an inference.

### Overflow, interrupt, and reset interval

In interval mode (`WT/IT=1`), overflow wraps TCWD and latches OVF. The interrupt
request is the level `WT/IT && IEOVF && OVF`, vector 31; it does not need a
second interrupt-controller flag or WDON to remain one. Clearing WDON does not
acknowledge an already pending interrupt. Multiple overflows with OVF already
one coalesce; advance count by arithmetic and do not queue an interrupt per
wrap. In watchdog mode, each overflow asserts reset regardless of IEOVF or
CCR.I, sets WRST, and then reset initialization clears OVF.
([§§12.3–12.4][wdt-operation], table 3.1, p. 43.)

The internal reset lasts **512 ROSC cycles**, irrespective of the source which
overflowed. Rev. 3 explicitly records this correction on p. 504; older related
H8 application notes saying 512 φOSC cycles must not override it. Represent a
reset assertion and a remaining ROSC-edge obligation, not immediate restart
or 512 CPU cycles. At assertion:

1. Abort current CPU execution/access continuation and initialize the MCU reset
   domain. Set WRST and retain it through the watchdog reset. The post-reset
   WDT registers are `F0/AF/57/00`.
2. Resolve changed GPIO/peripheral drives immediately. External chips retain
   power and ongoing work, but can observe reset-induced chip-select edges.
   Preserve RAM, the RTC's retained domain, and independent physical source
   phases according to their own reset rules.
3. Keep CPU execution and reset-vector fetch blocked throughout the internal
   reset hold. Keep resettable counters in reset; WDON's reset value does not
   allow TCWD to run during the hold.
4. At the 512th subsequent ROSC edge, release the internal reset contribution.
   The physical RES input can still hold the MCU in reset. Begin the normal
   reset exception sequence only when all reset sources are released.

The 512 count and reset-state/vector distinction are documented (§12.3.1,
[§3.2, pp. 44–45][wdt-reset-sequence]). Counting subsequent ROSC edges when
assertion occurs between them is the selected synchronization rule; preserve
that source phase. If ROSC was stopped, enable it and establish a new oscillator
phase at assertion. A reset from watch/standby also starts the main oscillator;
do not reuse the stopped oscillator's imaginary elapsed edges or add a SYSCR1
interrupt-wake STS delay to the separately specified watchdog reset interval
(§§5.2.2–5.2.4, pp. 88–89).

Keep the hold outside the register reconstruction that currently happens in
`Mcu::reset`, or explicitly retain it across that operation. Combine it with
the existing external `reset_asserted` condition in CPU scheduling. Advertise
the release appointment alongside external-device deadlines. A RES assertion
during this hold clears WRST; later watchdog-hold release must not set it again.
Power loss cancels the internal sequencer. These overlap rules follow independent
reset causes plus the explicit WRST clear condition.

### Qualified writes and the PC-alignment erratum

TCSRWD1/TCSRWD2 require MOV writes; bit-manipulation writeback cannot change
them. TMWD and TCWD do not carry that instruction restriction. For the two
control bytes, compute all qualifications from the **pre-write** state:

| Destination | Accepted change |
| --- | --- |
| TCSRWD1.TCWE | Write bit 7 as zero; copy written bit 6. |
| TCSRWD1.TCSRWE | Write bit 5 as zero; copy written bit 4. |
| TCSRWD1.WDON | Old TCSRWE=1 and written bit 3=0; copy bit 2. |
| TCSRWD1.WRST | Old TCSRWE=1 and written bits 1 and 0 both zero; clear only. Software cannot set WRST. |
| TCWD | Old TCWE=1; load the byte without resetting the source divider. |
| TCSRWD2.OVF | A real prior TCSRWD2 read observed OVF=1, and written bit 7=0; clear only. A debugger peek does not qualify. |
| TCSRWD2.WT/IT | Written bit 6=0; copy bit 5, subject to the clear erratum. |
| TCSRWD2.IEOVF | Written bit 4=0; copy bit 3, subject to the clear erratum. |

TCSRWD1 write-inhibit bits read `AA`; TCSRWD2 write-inhibit/reserved bits read
`57`; TMWD upper bits read `F0`. TCSRWE does not additionally protect TCSRWD2.
Enabling TCSRWE and clearing WDON in one write while old TCSRWE=0 therefore
does not stop the counter. A read of OVF=0 cannot authorize clearing an overflow
that occurs later. Successful OVF clear consumes its qualification; a read of
zero/reset removes it. A write preserving OVF need not consume the qualification.
([§12.2, pp. 203–207][wdt].)

[A309B/E][wdt-erratum] applies the clear problem **only in interval mode**.
The manual's concrete MOV.B absolute-8 example succeeds at instruction PC
`00A2`, fails at `0234`, and succeeds on retry at `023E`: PC bit 1 matters,
not the fixed address FFB2. Pass the issuing instruction's start PC and
addressing form with its actual bus write. Use the already decoded instruction;
do not reread ROM at that PC or mistake a split word store for MOV.B.

Chosen narrow model: if pre-write WT/IT=1 and an absolute-8 MOV.B's start PC
has bit 1 clear, inhibit only requested **1→0 transitions of WT/IT and IEOVF**.
Leave the other write qualifications effective. PC bit 1 set succeeds. No such
failure applies when setting those bits or in watchdog mode. Treat other MOV.B
addressing forms by the ordinary qualified-write rule until characterized; the
erratum does not supply a complete timing rule for them. Apply the mode test
even when WDON is zero, consistent with software's prescribed stop-before-mode
change sequence. These two restrictions are explicit modeling boundaries, not
claims that other encodings are immune on silicon.

Revision B's retry values are `87/97/C7` for clearing both/mode/enable,
respectively, with readback masks `28/20/08`; bit 7 stays one to preserve OVF.
The older A revision and rev. 3 manual table 12.2 show `07/17/47` instead.
Implement normal OVF qualification for **both**: after the retry read observes
OVF=1, the latter values can clear OVF as well. Do not reject the entire write
because its mode/enable clear fails.

### Live writes and compact chosen boundaries

Settle old-clock activity through the access before applying a write. A TMWD
change retains TCWD and OVF, then selects the new divider's existing phase;
switching to reserved 6/7 parks counting and switching back resumes without
catch-up. A live WT/IT change retains count and selects the action for the
**next overflow**, rather than converting old OVF into a retroactive reset.
The manual warns of count errors for running mode changes but supplies no
specific miscount; retained count is the selected nominal circuit behavior.
Accept the prohibited low-speed/interval combinations from §12.2.4 with their
selected source and ordinary flags instead of inventing a guest exception.

At an overflow coincident with a CPU write, use hardware-edge-before-access:
watchdog reset wins and aborts the access; interval-mode overflow first sets OVF
and wraps TCWD, then the qualified write applies. A TCWD service after the edge
can preload the counter but cannot undo OVF. A previously qualified OVF clear
can clear the coincident interval overflow. These are local race choices.
Ordinary WDON/module stops preserve OVF, count, and write qualification; actual
reset clears the qualifications. Keep qualified register writes usable in module
standby as a nominal bus-latch rule, so qualified WDON=1 can leave it without a
guest fault. No per-counter-edge global appointments are needed: advertise the
first reset/flag transition and the reset release; project reads arithmetically.

### Independent WDT fixtures

The following expected values come from register masks and rational source
counts, not the owner's helper functions. PC-race/private-divider choices are
identified above so they do not masquerade as hardware measurements.

| Stimulus | Expected result |
| --- | --- |
| Reset state, MOV.B writes `9E/A2/8E` to TCSRWD1 | Readbacks `BE/BA/AA`; only the second write stops TCWD. This is [`pw`'s WatchdogDisable][pw-watchdog]. |
| From AA, writes `9E/A6/8E`, then TMWD=F5 | Readbacks `BA/BE/AE`; counter enabled on φW/256. Starting count zero immediately after a selected edge needs 65,536 watch edges, exactly 2 s. TMWD is clock selection, not a reload byte. |
| From AE, [`pw` service][pw-watchdog-service]: `5E`, TCWD=00, `9E` | First and final TCSRWD1 read FE. The last write has B6WI=1 and therefore **does not relock TCWE**; do not infer different semantics from a helper's intent. |
| φ=4 MHz, CKS=15, TCWD=F1 immediately after an input edge | Overflow after 15×8192=122,880 φ cycles = 30.72 ms, reproducing the manual's approximate 30-ms example. |
| TCWD=F8 written just after a selected edge, CKS=5, φW=32,768 Hz | Eight selected edges until overflow = 2,048 watch cycles = 62.5 ms. Enter standby after three edges: count FB holds; five actual selected edges after resume remain. |
| CKS=8, count=10; enter watch, spend arbitrary watch time, then return active | TCWD remains 10. Prescaler S was cleared, so the first new /64 edge requires 64 active φ cycles after release, not an old global ordinal boundary. |
| Count=FE and WDON=1, clear WDCKSTP, supply two selected edges | Overflow still occurs. With WDON cleared first, both gates can be zero and count/OVF remain retained. |
| Interval OVF=0 was read; an overflow then sets it; write 7F to TCSRWD2 | OVF remains one. Read the one, then repeat 7F: OVF clears, while WT/IT and IEOVF are protected by written inhibit bits. |
| TCSRWD2=FF; MOV.B absolute-8 writes 87 at PC=0234 then 023E | First readback FF; second D7. OVF stays set while mode/enable clear on the successful site. No immediate reset is inferred from stale OVF. |
| TCSRWD2=FF, OVF was read as one; write 47 at failing PC=0234 | Readback 7F: OVF clears, IEOVF clear fails, WT/IT stays one. This distinguishes per-field erratum handling from rejecting the whole write. |
| Watchdog overflow exactly at ROSC ordinal `k` | Reset asserted once; CPU blocked through ordinal `k+511`, released at `k+512` absent another reset source. At the canonical ROSC rate the hold is 390.625 µs. Post-reset readbacks F0/AF/57/00. |
| Snapshot during that hold with 137 ROSC edges remaining | Restoration releases after those 137 edges, not a fresh 512; independent EEPROM/sensor deadlines still occur meanwhile. RES asserted before release changes WRST readback from AF to AE and prevents premature CPU restart. |

Primary caches: `out/research/h838602r-hardware.pdf/.txt`,
`h838606-addition.pdf/.txt`, and `h8-watchdog-a309b.pdf/.txt`. The firmware
references are pinned to public `pw` commit
`6dc7bc09950078fa3fe0dffa4dae34e9549a99da`; no WDT implementation or suite files
were changed by this research.

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

### ADC implementation checkpoint, 2026-09-18

The converter now uses the 31-step model above, including live source/channel
changes, retained open-mux charge, ten-CPU-cycle wake settling, and two sampled
ADTRG stages. PMRB/AMR/IEGR qualify the physical trigger; a trigger while ADSF is
already set does not restart it. A coincident completion clears ADSF after trigger
detection. Reset retains ADRR. Partition/restoration tests stop inside both the
trigger pipeline and the held-sample conversion.

Figure 17.6 (p. 358) places ideal quantization transitions at half-LSB boundaries;
§17.6 and table 21.7 specify the ±0.5-LSB quantization term. Conversion therefore
rounds `1024 * Vin / AVCC` and clips to 0..1023, rather than scaling by 1023.
AVCC is the external reference pin (§17.2, table 17.1), not an internal 3.3-V
reference. The current fixed reference condition remains a board-model placeholder
pending the separate battery-circuit investigation.

Independent Hachiware guests exercise all four clocks, open-mux retention, both
trigger edges, pin selection and interrupt vector 38. `out/adc-check` passed all
76 diagnostics and the workspace checks; `out/adc-retail` passed home/menu and
partition/restoration regression checks without changing their expectations.

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
[wdt-rosc]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=130
[wdt-prescalers]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=105
[wdt-electrical]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=441
[wdt-reset-sequence]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=78
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
[pw-watchdog-service]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/lib_common.c#L675-L681
[pw-reset]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L193-L209
