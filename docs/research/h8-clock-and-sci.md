# H8/38606 clocks and SCI3

Checked 2026-09-16 against the current starter's `mcu/clocks.rs`, `control.rs`, and
`sci.rs`. This note recommends implementation work; it does not change the core.

## Sources and applicability

- Renesas [H8/38602R hardware manual, REJ09B0152-0300, rev. 3.00][manual]:
  clock tree/registers/prescalers §§4.1–4.4, printed pp. 63–71; mode transitions
  §§5.1–5.3, pp. 78–94; SCI3 §§14.3–14.8, pp. 234–281. PDF page numbers are
  printed page numbers plus 34.
- [TN-H8*-A414A/E, addition of H8/38606][addition], 2009-04-22, pp. 1–5:
  target differences concern memory, package, and flash organization; it does not
  replace the clock/SCI chapters.
- [TN-H8*-A287A/E, specification changes][clock-update], 2004-11-10, p. 1:
  amended SYSCR1 stabilization guidance and STS table. Rev. 3 retains these values.
- [TN-H8*-A333B/E, SCI3 specification change][sci-update], 2006-07-07,
  pp. 2, 5–6: removes multiprocessor operation and specifies the 5-bit formats.
  Use revision B, not the superseded revision A. Rev. 3 incorporates the change.
- Public [`lumirth/pw`][pw] at `6dc7bc09950078fa3fe0dffa4dae34e9549a99da`.
  Firmware is evidence of exercised behavior, not a restriction on custom firmware.

## Hardware facts to preserve

| Area | Relevant facts and exact manual location |
| --- | --- |
| Sources | E7_2 selects the main oscillator at reset; OSCF is read-only. SUBSEL selects crystal watch clock or Rosc/32; SUBSTP stops the subclock oscillator. RFCUT controls feedback resistance, not clock selection. [§4.1.1, fig. 4.1, §4.2.4][clock-registers]. |
| Prescalers | S resets/stops in standby, watch, subactive, subsleep; W stops in standby but continues through watch/subactive/subsleep. [§4.4][prescalers]. |
| Transitions | SA changes take effect through SLEEP. Direct transitions include an intermediate sleep/watch state; I=1 prevents the direct-transition exception. Subactive→active includes STS delay counted in oscillator cycles, before destination-clock exception cycles. [§5.3, especially equation 6][direct]. |
| SCI clocks | CKS selects φ, φW, φ/16, φ/64. Internal bit periods: `(BRR+1)×32` source cycles, ×16 with ABCS, ×4 synchronous. External asynchronous clocks supply 16/8 samples per bit. Synchronous TX changes on falling SCK; RX samples rising SCK. [§§14.3.8–14.5][sci-clocks]. |
| SCI timing | Receiver start detection is clock-sampled. TEND/next TDR transfer occurs at stop-bit launch. TE enable first emits a mark frame. BRR initialization requires one bit interval. [§14.4.2–3, §14.8.4][sci-timing]. |

The STS selectors `000…111` correspond to
`8192, 16384, 1024, 2048, 4096, 256, 512, 16` oscillator states.
The update recommends `111` for an external/on-chip source and explicitly notes
different early-start behavior for other settings; a generic divided-CPU delay
does not express that distinction. [Clock update, p. 1][clock-update].

## Concrete consequences for this starter

1. **Represent clock control, not just frequency replacement.**
   `Control::write` rejects OSCCR bits 7/6, permits writes to OSCF, and stores
   SUBSEL without applying it. `select_clock` always uses `main_hz`/`watch_hz`.
   Implement those controls with the source distinctions above. Keep the board's
   reset strap as an input to reset selection; do not invent an OSCF-controlled
   runtime switch. [Current control](../../crates/hs-core/src/mcu/control.rs).

2. **Separate programmed divisors from the active transition state.**
   `select_clock` derives the active rate directly from SYSCR fields;
   `synchronize_clock` exposes that operation without retaining an independently
   latched selection. `Clocks::set_system` creates a new epoch at the supplied
   timestamp while retaining an ordinal. That cannot itself preserve the
   relationship to an already-running watch oscillator or express the different
   S/W reset rules. Retain physical source phases and explicit derived-clock
   state; commit programmed selections at the transition boundary.
   [Current clocks](../../crates/hs-core/src/mcu/clocks.rs).

3. **Give direct transitions a resumable stabilization obligation.**
   `Control::sleep` switches immediately and returns `true`, including departure
   from subactive mode. `wake` has a wait but counts it after installing the
   destination divider. For example, with MA selecting /8, its STS interval is
   multiplied by eight. Keep the existing STS lookup, attach it to the oscillator,
   and represent transition progress separately from interrupt entry. Preserve
   any applicable intermediate-mode reset effects. The caller must also account
   for CCR.I and the pre-transition internal processing state. [§5.3][direct].

4. **Use SCI edge obligations instead of captured wall-time periods.**
   `Tx`, `Rx`, and `pulse_end` retain absolute timestamps; `period()` captures a
   duration, and reception starts at the physical edge plus exactly half of it.
   Retain the baud-divider count/phase, sample index, and source instead. Project
   the next appointment from them. The existing CKS mapping and 32/16 factors
   are correct; keep them. Add SCK input/output and the synchronous framing branch
   to this same SCI owner. `validate_active` currently rejects both synchronous
   and external clocking, while accepted asynchronous CKE=01 has no SCK output.
   [Current SCI](../../crates/hs-core/src/mcu/sci.rs).

5. **Separate data availability, status changes, and line completion.**
   The current transmitter sets TEND and consumes the next holding byte only at
   frame completion, and starts immediately after TE/TDR setup. Add the required
   startup/stop-bit states without converting the transfer into an atomic byte.
   SCI module standby really resets its registers; ordinary absence of a selected
   clock must not be represented by that reset operation. The current
   `set_gate(false)` always reconstructs `Sci::default()`, so callers need to
   distinguish those causes. [§5.1.3, table 5.3][module-state].

6. **Correct the five-bit parity predicate.**
   `bits()` selects five data bits for MP=1, but `parity()` always follows PE.
   The documented MP=1, PE=1, CHR=0 format is five data bits **without parity**;
   CHR=1 adds parity. MP=1, PE=0 is prohibited, not an ordinary five-bit format.
   This follows the explicit corrected format table rather than the generic PE
   description. [SCI update, pp. 5–6][sci-update].

## Firmware evidence and inference

[`ClockSleep`][pw-sleep] writes SYSCR1/SYSCR2 then executes SLEEP.
[`CaptureSample`][pw-sample] uses `0xa7/0xeb` for a direct return from subactive
operation: STS=`010`, therefore 1024 oscillator states. This is a practical
regression case for the currently omitted wait, not a synthetic configuration.

[`IrConfigure`][pw-ir] enables SCI, selects SMR=0/BRR=0/SEMR=0, executes a short
settling loop, then enables RX and IrDA/TX. With the starter's 3.6864-MHz main
source, the derived baud rate is 115200. Its transmit helper polls TDRE and writes
TDR; a separate software TDRE-clear requirement would break real firmware.

For writes or transitions that violate software sequencing advice, use the
documented datapath as the starting model: preserve remaining source edges and
partial shift state unless an actual reset condition applies. That is an
engineering inference, not a claim that every such sequence has been measured.
Do not turn an unsupported implementation branch into a permanent guest fault
merely because the manual advises against a sequence. Keep any selected
tie-breaking rule local so later hardware evidence can refine it.

Useful focused checks are the `pw` direct transition, a transition into /8 mode,
start edges swept across the SCI sampling phase, external SCK stopped mid-byte,
TDR writes during the stop interval, the two five-bit formats, and exact
restoration mid-wait/mid-frame. Source-edge obligations belong in saved state;
cached appointment timestamps can be rebuilt.

## SCI3 implementation plan, 2026-09-17

This pass follows the target manual's actual timing drawings as well as its
text. Supplementary Renesas examples are REJ06B0247-0100Z, asynchronous
transmission; [REJ06B0371-0100Z, synchronous master reception][sync-example],
pp. 6, 12–14; and [REJ06B0431-0100, IrDA communication][irda-example], pp. 2–9.
Those examples corroborate the datapath, buffering, and receive-only master
clocking. Their different target parts do **not** override the H8/38606 clock
selectors, register bits, or removal of multiprocessor operation.

### Clock and state representation

Let `Q = BRR + 1` and let `S` be the CKS-selected source: `φ`, `φW`, `φ/16`,
or `φ/64`. The internal basic clock has a half-period of `Q` source edges.
Consequently:

| Operation | Clock obligation |
| --- | --- |
| Asynchronous, ABCS=0 | 32 half-basic-clock steps per bit: `32Q` source edges. |
| Asynchronous, ABCS=1 | 16 half-basic-clock steps per bit: `16Q` source edges. |
| Synchronous internal SCK | `2Q` source edges per half-SCK, `4Q` per bit. |
| Asynchronous external SCK | SCK **is** the basic clock: 16/8 complete input clock cycles per bit. Both edge polarities matter for reception. |
| Synchronous external SCK | SCK **is** the bit clock: output changes on falling edges, input is captured on rising edges. |

The first three formulas follow §§14.3.8, 14.3.11; the external modes follow
table 14.10 and §§14.5, 14.8.4. In asynchronous CKE=01, the SCK output runs at
the bit rate immediately upon clock selection, including while TE=RE=0; its
rising edge lies halfway through a transmit bit. In synchronous master mode,
SCK rests high and produces eight pulses per character. RE alone starts master
reception and its clock; RDRF does not pause it. Failure to drain RDR permits
overrun on a subsequent byte. [§§14.4.1–2][sci-timing],
[§§14.5.1–4][sci-sync], [Renesas receive example][sync-example].

Retain the BRC countdown and basic-clock polarity, a TX baud-divider phase,
and the RX sample position. Retain actual SCK/TXD output latches separately
from shift registers. An appointment is a projection of the remaining source
edges, not an authoritative `Duration` captured at frame start. Stable idle
input needs no recurring sample appointment: the next relevant input change
can be synchronized arithmetically to the next falling basic-clock edge.
External clocking advances only on resolved P30 edges; stopping SCK halfway
through a bit preserves everything until another edge arrives.

Ordinary CPU sleep leaves SCI operating. An unavailable selected source holds
the counters; it does not reset the peripheral or finish a frame in wall time.
Watch and standby instead initialize SMR/BRR/SCR/TDR/SSR/RDR/SEMR/IrCR, while
SPCR is retained. Explicit SCI module standby says all SCI registers enter
reset; apply that as a separate reset operation, including SPCR. The distinction
comes from §5.1.3 and the register-by-register [table 20.3, pp. 381–382][sci-reset].
Subactive/subsleep operation requires CPU clock φW (SA1=SA0=1), §14.8.9.
Keep this restriction separate from which oscillator exists: the manual also
advises against SCI with the on-chip oscillator (§14.8.10), but that advice is
not a hardware guest exception.

### Transmit and receive transitions

**Asynchronous transmit.** TE rising first emits a full frame of ones before
transmission becomes possible, as specified in figure 14.6, p. 260. Count
`1 + data_bits + parity_bits + stop_bits` cells. TDR writes while TE=1 clear
TDRE and TEND automatically; a second write before consumption replaces the
holding byte. Writing TDR does not require an earlier SSR read. At the **first
stop-bit launch**, consume a waiting TDR into TSR and set TDRE, or set TEND if
none is waiting. Preserve the entire one- or two-bit stop tail before starting
the next frame. TSR may therefore already contain the next byte while TXD is
still sending the preceding stop bit; a later TDR write queues another byte.
TE falling aborts transmission and restores TDRE/TEND to one. [§14.4.3,
figures 14.5–6][sci-timing], [§14.8.6][sci-pin-switch].

**Synchronous transmit.** Always eight data bits, LSB first, with no framing or
parity. The equivalent TDR-to-TSR/TEND decision is at D7's falling-edge launch.
The separate pin latch must still hold the outgoing D7 through its rising
sampling edge. Then SCK may stop high if neither transmitter nor receiver needs
another byte. TXD retains the last MSB. Receive error flags inhibit starting
another synchronous transmission. They do not inhibit asynchronous TX.
[§14.5.3][sci-sync], §14.8.3, p. 277.

**Asynchronous receive.** Figure 14.18, p. 278, places start synchronization on
the **falling basic-clock edge** and validation on the **eighth following rising
edge**. At 50% duty this is 7.5 basic periods after detection, not half a bit
after the unsynchronized physical transition. With ABCS=1, use the fourth
following rising edge, 3.5 basic periods. Validate that the start remains low;
otherwise return to idle without setting FER. Then sample data, optional parity,
and the first stop bit every 16/8 basic periods. Only the first stop is checked,
even in two-stop mode. A low line can initiate reception after RE is enabled or
errors are cleared; do not require a new physical falling edge, because a held
break produces FER repeatedly after software clears it. [§14.8.4,
figure 14.18][sci-sampling], §§14.3.5, 14.8.2, pp. 236, 277.

**Receive completion.** With RDRF=0, FER/PER still transfer the received data
to RDR but leave RDRF=0. With RDRF=1, overrun retains the old RDR and sets OER;
FER/PER can also be set. Any receive error blocks the next reception until
cleared. RDR reads clear RDRF automatically; RE=0 clears neither flags nor RDR.
Software zero-writes clear eligible SSR flags only after reading them as one;
one-writes cannot set them, and TEND is read-only. These are visible status
semantics, not reasons to drop bad bytes. [Table 14.11, p. 262][sci-rx-errors],
§§14.3.7, 14.8.7, pp. 239–242, 280.

### Formats, infrared, and board pins

Implement the corrected format table directly. With COM=0 and MP=0, CHR selects
8/7 data bits and PE enables parity. With MP=1 and PE=1, data length is five:
CHR=0 means **no parity**, CHR=1 means parity. PM=0 is even and PM=1 odd; STOP
selects one/two transmitted stop bits. COM=1 selects eight synchronous data
bits. The target has no multiprocessor/address filtering: SCR bit 3 is reserved,
SSR.MPBR always reads zero, and MPBT is reserved. [SCI update, pp. 2, 5–6][sci-update].

The IrDA block sits between UART and the pins. SCINV0 applies to the physical
input **before decoding**; SCINV1 applies **after encoding**. A zero UART bit
produces a positive pulse; a one produces none. Width selector 000 is three
basic-clock periods (3/16 of an ordinary asynchronous bit). Selectors
001/010/011/100 produce widths of **2/4/8/16 system-φ cycles**, independently
of CKS. Preserve those φ-edge obligations separately when the serial clock is
φW or changes. A stopped φ source can therefore stretch an already active
fixed-width pulse even if the serial source continues. ABCS=0 is the documented
IrDA configuration. [§14.3.10][sci-ir-width], [§14.6, figures 14.15–16][sci-irda].

The target figure places pulses near the middle of each bit; it does not
dimension the launch phase or describe the decoder's internal counter. Use
these compact boundary rules: launch a zero's pulse after **13 half-basic-clock
steps**, then hold it for six such steps for selector 000, or for the selected
φ count. The default pulse is thereby centered at the bit midpoint. On receive,
a positive pulse after SCINV0 sets decoded UART input low for one bit's basic
clock count; another positive pulse retriggers that hold. Treat a continuously
active input as low, too. Feed this decoded level through the same UART
start-validation/data sampler. These are selected circuit-level inferences,
not dimensions supplied by the drawing. They retain pulse phase, reception
latency, and clock stoppage without substituting completed bytes. Do **not**
reject pulses narrower than 1.41 µs: §14.6.2 explicitly recognizes them as zero.

Resolve the pins before translating them into optical behavior:

| Pin | Selection and observable behavior |
| --- | --- |
| P32 | SPC3=0 selects PCR32/PDR32 GPIO. SPC3=1 selects SCI or IrDA output according to IrE, irrespective of TE or PCR32. SCINV1 changes the selected peripheral signal immediately. |
| P31 | RE=0 selects GPIO; RE=1 selects SCI/IrDA input irrespective of PCR31. Inversion changes the SCI input immediately, not raw PDR3 input readback. |
| P30 | IRQ0S=`10` wins, then VCref. Otherwise CKE=00 uses GPIO in async mode and SCK output in sync mode; CKE=01 selects SCK output; CKE=10 selects SCK input. |

These are [§8.2.5, p. 127][sci-pins] and the pin circuits in appendix B.2,
pp. 476–478. PDR3 reads its output latch wherever PCR3=1, irrespective of the
actual alternate-function pin voltage (§8.2.1, p. 125). Switching synchronous
SCK output directly to GPIO produces a documented **half-φ-cycle low pulse**.
The prescribed intermediate external-clock selection avoids it; retain that
physical glitch rather than treating a mux write as electrically invisible.
[§14.8.5, p. 279][sci-pin-switch].

[`pw` initializes PCR3=5 and PDR3=1][pw-ir-pins], uses SCINV0=1 with
noninverted IrDA output, then [clears PDR3 to enable IR][pw-ir-start] and
[sets it on completion][pw-ir-finish]. This supports active-high P30 shutdown,
active-low receiver output on P31, and active-high transmit drive on P32.
Consequently GPIO can drive these same wires, and choosing SCK on P30 can toggle
the transceiver enable. Do not wire logical UART zero directly to light output;
normal UART mark is a high electrical P32 level. This inference establishes
digital polarity, not an unresearched analog pulse limiter or transceiver model.

### Small rules for live reconfiguration

The normal operation above is specified. The manual advises initialization with
TE=RE=0 and a one-bit BRR settling interval; it does not prescribe every
mid-frame register-write race. Complete the state machine with these local rules:

- Retain the current BRC countdown across CKS changes; consume remaining counts
  on the new source's next real edges. A BRR write supplies the next reload.
  Neither action restarts a frame or creates a synthetic clock edge.
- CKE changes select future internal/external clock events while retaining bit
  and sample position. COM/format/ABCS changes are retained for the next
  character; latch active format when TX loads TSR or RX accepts its start.
  Clock/pin selection remains live. A synchronous receiver starts a character
  on its first sampling edge. This avoids reinterpreting an existing shift word
  as a new frame while keeping the actual clock and pins observable.
- Start the TE mark counter on the next eligible TX bit boundary and consume
  a full frame of cells; it need not run global events for an unchanged mark
  level. A late TDR write during an unfilled stop tail may preload TSR, but
  cannot shorten that tail. Latch the pulse-width selector when a pulse starts.
  IrE and SPCR are live mux controls; TE=0 aborts TX, RE=0 aborts RX.
- Commit receive data and accumulated FER/PER at the first stop sample. Mask
  unused upper RDR bits to zero for five/seven-bit reception. A same-timestamp
  hardware completion precedes the CPU's bus observation, so an RDR read can
  consume the newly completed byte as described by figure 14.19.
- Keep reserved encodings total and simple: MP=1/PE=0 uses five bits without
  parity; reserved synchronous format bits have no effect; CKE=11 takes external
  input; unused IrCKS 101–111 produce no encoded pulse. Preserve writable fields
  for readback and hardwire reserved read bits as specified. These are chosen
  decoder rules, not additional claimed operating modes or guest faults.

### Independently expected fixtures

Each row derives from a register table, timing diagram, firmware sequence, or an
explicitly named boundary rule above. Source-edge counts are exact; rounded
microseconds are explanatory. Avoid deriving expected values by calling the
implementation's own framing or clock helpers.

| Stimulus | Expected observation |
| --- | --- |
| φ=3,686,400 Hz, CKS=00, BRR=0, ABCS=0 | Basic period is 2φ cycles; bit period is 32φ cycles = 8.680556 µs, 115200 baud. A default IR pulse lasts 6φ cycles = 1.627604 µs. |
| Same clock, 8N1, TE startup begins on boundary 0 and TDR waits throughout mark | Mark consumes 320φ cycles. Start is at cycle 320; absent another byte, TEND rises at cycle 608, and stop tail ends at 640. The exact first-boundary convention is the selected rule above. |
| Send A5 as 8N1, start at `t=0` | Bit cells are `0 | 1 0 1 0 0 1 0 1 | 1`. TEND/next holding transfer occurs at `9B`; next start cannot precede `10B`. Two-stop mode moves that next start to `11B`, not the transfer decision. |
| Write byte B before byte A's first stop, then byte C during that stop | B has already moved to TSR and TDRE=1 at stop launch. C fills TDR; it must not replace B. |
| SMR=24/64/74 hex, send F5 | Five low data bits are `1 0 1 0 1`. Respectively produce 5N1 with no parity, 5E1 with parity 1, and 5O1 with parity 0. |
| Basic-clock rises at integer times, falls at `k+0.5`; RX falls at 0.6 and remains a valid 8N1 frame | Detect at 1.5, validate start at 9, sample D0 at 25, D7 at 137, stop/commit at 153. Moving the fall to 0.4 moves these to 0.5, 8, 24, 136, 152. A low glitch from 0.6 through 8.9 is rejected at 9. |
| Receive A5 with bad parity, initially empty RDR and otherwise reset SSR | RDR=A5, SSR=8C (PER, TDRE, TEND), RDRF=0. Bad stop without parity error gives SSR=94. Clear errors, hold normal UART RX low: reception resumes and eventually reports another FER. |
| RDR=A5/RDRF=1; another valid byte completes before read | RDR remains A5; SSR=E4. If the new byte also has a bad stop, SSR=F4. Reading RDR clears RDRF but leaves OER/FER. |
| External synchronous SCK, transmit A5, receive 3C | Falling edges output `1 0 1 0 0 1 0 1`; rising edges sample `0 0 1 1 1 1 0 0`. TEND can assert on the eighth falling edge; RDR=3C/RDRF=1 after the eighth rising edge. Pause anywhere: no autonomous ninth edge or data completion. |
| Live CKS change with one BRC source edge remaining | The next edge of the newly selected physical tap completes that half-basic-clock obligation; it does not restart `Q` counts. This tests the selected reload/mux rule. |
| IrCKS=001/010/011/100 at 3.6864 MHz | Widths are 2/4/8/16φ cycles: 0.542535/1.085069/2.170139/4.340278 µs. For selector 000 and zero launched at cycle 0, the selected centered model drives high at cycle 13 and low at 19. |
| SPCR.SPC3=1, idle normal UART, flip SCINV1 | Physical P32 flips immediately. Clear SPC3 with PCR32=1: P32 follows PDR32, independently of TE. Readback with PCR32=1 still returns the port latch. |
| Switch synchronous SCK output directly to GPIO at 3.6864 MHz | The documented transient is low for half a φ cycle = 0.135634 µs; the three-step §14.8.5 sequence avoids it. |

The cached primary documents are `out/research/h838602r-hardware.pdf/.txt`,
`sci3-change.pdf/.txt`, and `h8-sci-{async,master-rx,irda}-apn.pdf/.txt`.
Rendered `sci-sampling-detail.png` and `sci-irda-272.png` preserve the figures
used for edge/pulse interpretation. These caches are ignored; the links below
remain the source references.

[manual]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[addition]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=2
[clock-update]: https://www.renesas.com/en/document/tcu/h838602-group-specification-changes#page=2
[sci-update]: https://www.renesas.com/us/en/document/tcu/about-sci3-specification-change-0#page=6
[clock-registers]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=97
[prescalers]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=105
[direct]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=125
[sci-clocks]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=277
[sci-timing]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=292
[module-state]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=115
[pw]: https://github.com/lumirth/pw/tree/6dc7bc09950078fa3fe0dffa4dae34e9549a99da
[pw-sleep]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/lib_common.c#L658-L672
[pw-sample]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_main.c#L517-L523
[pw-ir]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/ir.c#L123-L150
[sci-sync]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=299
[sci-sampling]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=312
[sci-rx-errors]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=296
[sci-ir-width]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=286
[sci-irda]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=305
[sci-pins]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=161
[sci-pin-switch]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=313
[sci-reset]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=415
[sync-example]: https://www.renesas.com/en/document/apn/clock-synchronization-serial-data-master-reception#page=8
[irda-example]: https://www.renesas.com/in/en/document/apn/h8300h-slp-series-application-note-infrared-communication-using-irda#page=4
[pw-ir-pins]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/ir.c#L65-L70
[pw-ir-start]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/ir.c#L158-L180
[pw-ir-finish]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/ir.c#L1150-L1160
