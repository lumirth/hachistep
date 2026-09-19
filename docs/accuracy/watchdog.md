# Watchdog

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

The watchdog implements protected register writes, count sources, interval/reset modes,
overflow flag qualification and its 512-ROSC reset hold. The hold combines with RES
qualification rather than releasing an outstanding reset. Source and prescaler lifetime
follow the operating mode. The matching firmware exercises the disable/service sequences.

## Limits and open questions

Revision B of TN-H8*-A309 describes an instruction-address-dependent register-write
defect. The implementation applies it to the relevant MOV.B absolute-8 writes and
fields. This is a silicon rule attached to the actual access. Review unusual instruction
forms and simultaneous overflow/clear/reset against this erratum and the manual before
changing their behavior. A passing ordinary service sequence covers only one use.

## Sources and applicability

The H8/38606 target addition leaves these peripherals unchanged. The applicable baseline
is REJ09B0152-0300 rev. 3.00. Page references are printed manual pages; PDF pages are 34
higher. ([Target applicability][target], [manual][manual])

## Watchdog

The target addition does not replace WDT behavior. Use
[manual §§12.1–12.5, pp. 201–211][wdt] and TN-H8*-A309B/E, rev. 2, 2005-10-04,
rather than its superseded A revision.

### Counter source, gates, and retained phase

`FFB0–FFB3` are TMWD/TCSRWD1/TCSRWD2/TCWD, byte-wide registers with two-state bus
accesses (table 20.1, p. 374). Reset values are `F0/AE/57/00`; WDON starts enabled. TCWD
is an eight-bit up-counter, and the transition `FF→00`, not reaching FF, causes
overflow. TMWD selects:

| CKS | Source | Full 256-count interval with canonical frequencies |
| --- | --- | --- |
| 0–3 | ROSC/2048, documented aliases | 400 ms at ROSC=1,310,720 Hz |
| 4 | φW/16 | 125 ms at 32,768 Hz |
| 5 | φW/256 | 2 s at 32,768 Hz |
| 6–7 | Reserved | Retain encoding; disconnect counter clock as the selected rule. |
| 8–15 | φ/64, /128, /256, /512, /1024, /2048, /4096, /8192 | 4.444444 ms through 568.888889 ms at φ=3,686,400 Hz |

The ROSC frequency is a unit parameter; the default reproduces the manual's 0.4-s
typical overflow period, not a precision-oscillator guarantee ([table 21.9, p.
407][wdt-electrical]). A TCWD preload `x` leaves exactly `256−x` selected input edges,
while the first edge retains the source's phase.

WDON gates counting. Effective module operation is `WDON || WDCKSTP`: clearing WDCKSTP
while WDON=1 records the bit but cannot stop TCWD. Once WDON becomes zero, WDCKSTP=0
takes effect. Module standby retains registers; it does not reset or clear pending OVF.
The shared source has separate availability:

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

Preserving the oscillator is not the same as preserving every divider. Prescaler S
resets to zero on MCU reset and on entering watch, subactive, subsleep, or standby.
Prescaler W resets on MCU reset, holds in standby, and continues through
watch/subactive/subsleep. Both start counting on reset release. These are explicit
[§4.4, p. 71][wdt-prescalers] rules. System/watch WDT taps must use those shared
dividers, not `global_source_ordinal/divisor` across every transition. For the
undocumented private ROSC/2048 divider, choose the corresponding small circuit model:
retain its phase across WDON changes and ordinary source selection, hold it when its
module/source stops, and clear it with MCU reset. Treat that private-divider reset
placement as an inference.

### Overflow, interrupt, and reset interval

In interval mode (`WT/IT=1`), overflow wraps TCWD and latches OVF. The interrupt request
is the level `WT/IT && IEOVF && OVF`, vector 31; it does not need a second
interrupt-controller flag or WDON to remain one. Clearing WDON does not acknowledge an
already pending interrupt. Multiple overflows with OVF already one coalesce; advance
count by arithmetic and do not queue an interrupt per wrap. In watchdog mode, each
overflow asserts reset regardless of IEOVF or CCR.I, sets WRST, and then reset
initialization clears OVF. ([§§12.3–12.4][wdt-operation], table 3.1, p. 43.)

The internal reset lasts 512 ROSC cycles, irrespective of the source which overflowed.
Rev. 3 explicitly records this correction on p. 504; older related H8 application notes
saying 512 φOSC cycles must not override it. Represent a reset assertion and a remaining
ROSC-edge obligation, not immediate restart or 512 CPU cycles. At assertion:

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
assertion occurs between them is the selected synchronization rule; preserve that source
phase. If ROSC was stopped, enable it and establish a new oscillator phase at assertion.
A reset from watch/standby also starts the main oscillator; do not reuse the stopped
oscillator's imaginary elapsed edges or add a SYSCR1 interrupt-wake STS delay to the
separately specified watchdog reset interval (§§5.2.2–5.2.4, pp. 88–89).

The reset hold survives MCU register initialization and prevents CPU execution until
both internal and external reset sources release. A RES assertion during this hold
clears WRST; later watchdog-hold release must not set it again. Power loss cancels the
internal sequencer. These overlap rules follow independent reset causes plus the
explicit WRST clear condition.

### Qualified writes and the PC-alignment erratum

TCSRWD1/TCSRWD2 require MOV writes; bit-manipulation writeback cannot change them. TMWD
and TCWD do not carry that instruction restriction. For the two control bytes, compute
all qualifications from the pre-write state:

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

TCSRWD1 write-inhibit bits read `AA`; TCSRWD2 write-inhibit/reserved bits read `57`;
TMWD upper bits read `F0`. TCSRWE does not additionally protect TCSRWD2. Enabling TCSRWE
and clearing WDON in one write while old TCSRWE=0 therefore does not stop the counter. A
read of OVF=0 cannot authorize clearing an overflow that occurs later. Successful OVF
clear consumes its qualification; a read of zero/reset removes it. A write preserving
OVF need not consume the qualification. ([§12.2, pp. 203–207][wdt].)

[A309B/E][wdt-erratum] applies the clear problem only in interval mode.
The manual's concrete MOV.B absolute-8 example succeeds at instruction PC `00A2`, fails
at `0234`, and succeeds on retry at `023E`: PC bit 1 matters, not the fixed address
FFB2. Pass the issuing instruction's start PC and addressing form with its actual bus
write. Use the already decoded instruction; do not reread ROM at that PC or mistake a
split word store for MOV.B.

Chosen narrow model: if pre-write WT/IT=1 and an absolute-8 MOV.B's start PC has bit 1
clear, inhibit only requested 1→0 transitions of WT/IT and IEOVF. Leave the other write
qualifications effective. PC bit 1 set succeeds. No such failure applies when setting
those bits or in watchdog mode. Treat other MOV.B addressing forms by the ordinary
qualified-write rule until characterized; the erratum does not supply a complete timing
rule for them. Apply the mode test even when WDON is zero, consistent with software's
prescribed stop-before-mode change sequence. These two restrictions are explicit
modeling boundaries, not claims that other encodings are immune on silicon.

Revision B's retry values are `87/97/C7` for clearing both/mode/enable, respectively,
with readback masks `28/20/08`; bit 7 stays one to preserve OVF. The older A revision
and rev. 3 manual table 12.2 show `07/17/47` instead. Implement normal OVF qualification
for both: after the retry read observes OVF=1, the latter values can clear OVF as well.
Do not reject the entire write because its mode/enable clear fails.

### Live writes and compact chosen boundaries

Settle old-clock activity through the access before applying a write. A TMWD change
retains TCWD and OVF, then selects the new divider's existing phase; switching to
reserved 6/7 parks counting and switching back resumes without catch-up. A live WT/IT
change retains count and selects the action for the next overflow, rather than
converting old OVF into a retroactive reset. The manual warns of count errors for
running mode changes but supplies no specific miscount; retained count is the selected
nominal circuit behavior. Accept the prohibited low-speed/interval combinations from
§12.2.4 with their selected source and ordinary flags instead of inventing a guest
exception.

At an overflow coincident with a CPU write, use hardware-edge-before-access: watchdog
reset wins and aborts the access; interval-mode overflow first sets OVF and wraps TCWD,
then the qualified write applies. A TCWD service after the edge can preload the counter
but cannot undo OVF. A previously qualified OVF clear can clear the coincident interval
overflow. These are local race choices. Ordinary WDON/module stops preserve OVF, count,
and write qualification; actual reset clears the qualifications. Keep qualified register
writes usable in module standby as a nominal bus-latch rule, so qualified WDON=1 can
leave it without a guest fault. No per-counter-edge global appointments are needed:
advertise the first reset/flag transition and the reset release; project reads
arithmetically.

### Independent WDT fixtures

The following expected values come from register masks and rational source counts.
Cases involving interrupt races and the private divider use the model choices above.

| Stimulus | Expected result |
| --- | --- |
| Reset state, MOV.B writes `9E/A2/8E` to TCSRWD1 | Readbacks `BE/BA/AA`; only the second write stops TCWD. This is [`pw`'s WatchdogDisable][pw-watchdog]. |
| From AA, writes `9E/A6/8E`, then TMWD=F5 | Readbacks `BA/BE/AE`; counter enabled on φW/256. Starting count zero immediately after a selected edge needs 65,536 watch edges, exactly 2 s. TMWD is clock selection, not a reload byte. |
| From AE, [`pw` service][pw-watchdog-service]: `5E`, TCWD=00, `9E` | First and final TCSRWD1 read FE. The last write has B6WI=1 and therefore does not relock TCWE; do not infer different semantics from a helper's intent. |
| φ=4 MHz, CKS=15, TCWD=F1 immediately after an input edge | Overflow after 15×8192=122,880 φ cycles = 30.72 ms, reproducing the manual's approximate 30-ms example. |
| TCWD=F8 written just after a selected edge, CKS=5, φW=32,768 Hz | Eight selected edges until overflow = 2,048 watch cycles = 62.5 ms. Enter standby after three edges: count FB holds; five actual selected edges after resume remain. |
| CKS=8, count=10; enter watch, spend arbitrary watch time, then return active | TCWD remains 10. Prescaler S was cleared, so the first new /64 edge requires 64 active φ cycles after release, not an old global ordinal boundary. |
| Count=FE and WDON=1, clear WDCKSTP, supply two selected edges | Overflow still occurs. With WDON cleared first, both gates can be zero and count/OVF remain retained. |
| Interval OVF=0 was read; an overflow then sets it; write 7F to TCSRWD2 | OVF remains one. Read the one, then repeat 7F: OVF clears, while WT/IT and IEOVF are protected by written inhibit bits. |
| TCSRWD2=FF; MOV.B absolute-8 writes 87 at PC=0234 then 023E | First readback FF; second D7. OVF stays set while mode/enable clear on the successful site. No immediate reset is inferred from stale OVF. |
| TCSRWD2=FF, OVF was read as one; write 47 at failing PC=0234 | Readback 7F: OVF clears, IEOVF clear fails, WT/IT stays one. This distinguishes per-field erratum handling from rejecting the whole write. |
| Watchdog overflow exactly at ROSC ordinal `k` | Reset asserted once; CPU blocked through ordinal `k+511`, released at `k+512` absent another reset source. At the canonical ROSC rate the hold is 390.625 µs. Post-reset readbacks F0/AF/57/00. |
| Snapshot during that hold with 137 ROSC edges remaining | Restoration releases after those 137 edges, not a fresh 512; independent EEPROM/sensor deadlines still occur meanwhile. RES asserted before release changes WRST readback from AF to AE and prevents premature CPU restart. |

## Firmware reset cause

[`pw` startup](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L193-L209)
reads WRST and updates an EEPROM diagnostic counter. Reset-cause preservation therefore
affects later firmware writes. The disable and service sequences are included in the
fixture table above.

## Implementation and checks

The [watchdog](../../crates/hs-core/src/mcu/watchdog.rs) owns qualified writes and
reset hold; [power and reset](power-and-reset.md) explains how reset causes combine.
Hachiware's [watchdog cases](https://github.com/lumirth/hachiware/blob/main/cases/watchdog.py)
check control qualification, instruction alignment and overflow read qualification.
Local [watchdog tests](../../crates/hs-core/tests/watchdog.rs) exercise timing and
reset interaction. The erratum establishes a defect, while the exact treatment of
unmentioned instruction forms remains bounded as described above.

[manual]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[target]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=2
[wdt]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=237
[wdt-operation]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=242
[wdt-erratum]: https://www.renesas.com/en/document/tcu/h838086r-group-h838076r-group-h838602r-group-watchdog-timer-usage-note-0#page=2
[wdt-power]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=120
[wdt-rosc]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=130
[wdt-prescalers]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=105
[wdt-electrical]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=441
[wdt-reset-sequence]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=78
[pw-watchdog]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/lib_common.c#L582-L598
[pw-watchdog-service]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/lib_common.c#L675-L681
