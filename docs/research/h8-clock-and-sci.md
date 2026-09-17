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
