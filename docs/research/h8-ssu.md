# H8 SSU: transfer state, clock changes, and firmware use

`CaptureSample` writes a new byte and then changes SSMR at `0xf0e2` without
another TEND wait. Its intervening CPU work must run under the correct clock.
If a custom program changes CKS during a transfer, continue the existing shifter
under the new clock selection. Do not complete the byte, restart it, or defer an
intervening chip-select change.

This note uses REJ09B0152-0300 Rev.3.00, §15, printed pp.283–310
([original manual][manual]); add 34 for PDF page numbers. H8/38606 applicability
is established in [h8-execution.md](h8-execution.md). Firmware links are pinned to
`lumirth/pw` commit `6dc7bc09950078fa3fe0dffa4dae34e9549a99da`.
Local source copies and rendered timing figures are in ignored `out/research/`.
No hardware measurement or implementation was performed for this note.

## The actual reached sequence

[`CaptureSample`, lines 506–522][pw-capture] selects subclock/2, selects the BMA150,
writes its control-register address, and waits for TEND. It then writes the awake
value, services the watchdog, writes SYSCR1/SYSCR2, changes the firmware clock
flag, deselects the sensor, restores main-clock/4, and executes SLEEP. There is
no second TEND poll between the data write and deselection/clock change.

The implementing agent's instruction trace identified the second SSTDR write at
PC `0x76d8` and the SSMR write at `0x76fc`, with only about 15 microseconds between
them. This trace is an emulator observation, not evidence that silicon uses the
same timing. At 32,768 Hz, subclock/2 needs approximately 488 microseconds for
eight bits. Consequently, CPU clock-transition timing can determine whether
the sensor receives the complete awake command before deselection.

The integration defect was resolved in `Machine::queue_cpu`: a newly completed
SLEEP prefetch had set the CPU's sleeping state before the MCU entered its
programmed mode. The wake guard then skipped that mode transition. Checking
both states restores the subclock execution interval, and the retail sequence
completes without resets. `hachiware`'s `direct-clock-transitions` guest covers
the handoff independently. Live CKS continuation is still useful for custom
firmware; the failed trace does not establish that retail needs it.

Other reached uses establish distinct contracts:

- [`AccelRead`/`AccelWrite`][pw-accel] use TDRE to queue bytes, RDRF to consume
  received bytes, and TEND before deselecting the device. Those flags are not
  interchangeable.
- [`EepromConfigure`/`EepromIdle`][pw-eeprom] disable the SSU before initial clock
  selection, but restore transmit-only mode and main-clock/4 without clearing
  TE. Ordinary EEPROM tails wait for TEND before deselection.
- [`DisplaySend`][pw-display] selects the LCD, waits for TDRE, writes one byte,
  waits for TEND, and deselects. All emitted clock edges must reach the shared
  board wiring, including edges when no device is selected.

## Documented transfer and flag rules

The [register descriptions, §§15.3.3–15.3.8, pp.289–293][registers] establish:

| State/change | Required behavior |
| --- | --- |
| SSTDR write | Update the holding register; clear TDRE and TEND. Do not overwrite the active shift register. |
| Holding-to-shift transfer | Set TDRE; this means another byte can be queued, not that transmission ended. |
| Last transmitted bit with TDRE=1 | Set TEND. Queued data permits continuous transmission. |
| Eighth received bit with RDRF=0 | Copy the completed byte to SSRDR and set RDRF. |
| Eighth received bit with RDRF=1 | Preserve the older SSRDR byte, set ORER, lose the newly received byte. |
| SSRDR read | Return SSRDR and clear RDRF; in receive-only master mode this is also a start/restart trigger. |
| TE cleared | Set TDRE. |
| RE cleared | Preserve RDRF, ORER, and SSRDR; see the explicit initialization notes on pp.297 and 304. |
| Software status clear | Clear only a flag previously read as one and then written as zero. |

ORER blocks further reception; in master mode it also blocks transmission.
The five interrupt sources are level conditions formed from their own status
and enable bits, sharing vector 34. Clearing TDRE in software can request an
additional transmission of the existing SSTDR value; a separate `holding`
option must not override that hardware state. [§§15.3.5, 15.4.11][flags]

TEND is sticky until its documented clear conditions occur. Merely loading the
shifter is not one of them; the starter's unconditional TEND clear in `Load`
would lose this distinction when software requests a repeat by clearing TDRE.

SSTDR is readable and writable at all times. A second write before its pending
byte transfers therefore replaces that pending value. It does **not** create
CE. CE belongs to SCS arbitration or deselection during a slave transfer.
The current `holding.is_some()` → `status |= 1` branch invents a conflict error.
[§§15.3.5, 15.3.7, 15.4.10][arbitration]

## Clock edges and completion

CPOS is the inverse of conventional SPI CPOL; CPHS is the inverse of conventional
SPI CPHA. The definitions and [Figure 15.2, p.294][phase] give:

| CPOS | CPHS | Idle SSCK | Data changes | Data samples | SPI mode |
| ---: | ---: | --- | --- | --- | ---: |
| 0 | 0 | High | Falling | Rising | 3 |
| 0 | 1 | High | Rising; first bit preloaded | Falling | 2 |
| 1 | 0 | Low | Rising | Falling | 1 |
| 1 | 1 | Low | Falling; first bit preloaded | Rising | 0 |

Count the first edge away from idle as edge 1. With CPHS=0, the eighth sample is
edge 16. With CPHS=1, it is edge 15: **RDRF/ORER can be asserted while the final
return-to-idle edge is still pending.** The manual expressly calls this out in
[§15.4.9 and Figure 15.12, pp.307–308][receive]. Receiving the last bit, consuming
SSRDR, finishing the frame, and releasing SCS must remain separate transitions.

[Figure 15.11, p.306][transmit] distinguishes the final-bit TEND indication from
the later frame/SCS boundary. For the reached CPHS=0 path, completing the eighth
sample at the final rising edge is consistent with the diagram. Its drawing
does not establish a system-clock-exact TEND synchronization delay for every
CPHS setting. Keep that flag boundary independently representable; do not add
an arbitrary whole-bit delay or merge TEND with TDRE. The runtime failure above
does not prove an early TEND bug: a **new SSTDR write follows the successful
TEND poll** and clears it again.

Likewise, the manual specifies when TDRE becomes available relative to loading
the shifter, but does not justify the starter's particular one-system-clock
load appointment as a measured latency. Preserve separate load and shift
effects, and check start/continuous-transfer edges against the diagrams.

The text on p.305 says SSCK finishes high while discussing an idle-high example.
It cannot override CPOS's explicit idle-low setting and Figure 15.2. Finish at
the configured idle level and retain the last transmitted output bit.

## Live writes: a concrete continuation rule

The [initialization procedures, pp.297 and 304][initialization] require clearing
TE/RE before changing operating mode or transfer format. They do not define a
CPU exception, emulator halt, or automatic byte completion for another order.
The hardware model therefore still needs a physical continuation for such writes.

For a **CKS-only change** such as `0x87` → `0x86`:

1. Settle old-clock events through the MMIO write's established timestamp.
2. Mask reserved bits before comparing the effective old/new settings.
3. Retain the shift contents, bit/edge counts, sampled input, queued byte,
   status flags, and driven clock/data levels.
4. Replace the pending edge's source/divider while preserving its remaining
   edge obligation. Use the new source's existing phase. Do not preserve the
   old wall-time delay or invent a fresh oscillator phase.
5. If that source is stopped or the module is gated, retain the obligation until
   clocking resumes. A separate holding-load appointment is not a serial edge.

This is an explicit **live-multiplexer inference** from the documented clock
selector, rather than a documented guarantee about active-switch glitches.
It is deterministic, fits the reached firmware, and preserves completed effects.
A CKS-only write itself must not force SSCK to idle or emit an extra edge.

Other active changes must also preserve completed effects. A practical model is
to apply CPOS to the output-polarity mux immediately, route any resulting pin
transition through the board, and apply CPHS to subsequent sampling/shift
decisions without resetting the bit count. The exact behavior outside the
initialization sequence is inferred. MLS deserves separate handling: §15.3.8
describes its mapping when SSTDR transfers to SSTRSR. Retain already-loaded
shift contents rather than revisiting the original source byte with a newly
reversed index partway through a transfer.

## Receive-only, slave, and bidirectional operation

- **Receive-only master:** setting RE alone does not start clocks. A dummy
  SSRDR read starts reception. With RSSTP clear, reception continues across
  byte boundaries; unread data can cause ORER. With RSSTP set, stop after the
  current byte's complete clock sequence. Do not auto-clear RE. Firmware clears
  RE and RSSTP before reading the final byte because reading while RE remains
  set can start another frame. Full-duplex transfers instead start from SSTDR.
  [§15.4.5, pp.300–302; §15.4.9][receive-only]
- **Slave:** use resolved external SSCK edges; do not schedule master-clock
  deadlines. Four-line mode is selected while SCS is low. A high transition
  during a frame sets CE and terminates that selected transfer. Idle without
  an external clock is ordinary hardware behavior, not `Unsupported`.
  [§§15.3.5, 15.4.1, 15.4.8–15.4.10][arbitration]
- **Pin routing:** in synchronous mode, SSI receives and SSO transmits for
  either master or slave. In four-line normal mode, a slave receives on SSO and
  transmits on SSI. In bidirectional mode, SSO carries both directions, selected
  by receive/transmit enable; the documented combinations enable one direction
  at a time. BIDE is ignored when SSUMS=0. Preserve the same shifter and board
  edge path; change pin direction/routing instead of adding another executor.
  [§15.4.3/Figure 15.3 and Table 15.2, pp.295–296][pins]

CSS=00 leaves SCS under GPIO control, as in the retail configuration `SSCRH=0x8c`.
Hardware-controlled SCS requires its own arbitration: a low synchronized SCS
before a master transfer sets CE and clears MSS. Honor output release and
open-drain selection; an inactive SSU output is not a driven Boolean value.

## Corrections after the live-clock fix

In [the SSU implementation](../../crates/hs-core/src/mcu/ssu.rs), prioritize:

1. Remove the RE-disable clearing of RDRF/ORER; preserve unread data and errors.
2. Make pending SSTDR/TDRE semantics agree, including overwrite and explicit
   status clears. Reserve CE for its documented SCS conditions.
3. Implement receive-only triggering/stopping and remove the automatic RE clear
   on RSSTP. Keep the last required clock edge after an early receive flag.
4. Make SRES reset the sequencer while retaining SSU registers, as §15.3.2 p.288
   specifies. `status = 4` on SRES incorrectly resets a retained register.
5. Route slave/bidirectional pins through the existing board. Implement SOL
   readback and protected writes: SOL reads the actual serial output level;
   SOLP reads one, and a zero write permits an output-level change. The target's
   §15.5 p.310 documents a protection-transition quirk; do not reduce these bits
   to ordinary stored configuration.

Distinguishing cases should cover CKS writes on either side of an edge; CS rising
mid-byte; CPHS=1 receive completion before the trailing edge; flag retention on
RE disable; overwritten pending SSTDR; TDRE cleared without an SSTDR write;
RSSTP followed by an SSRDR read with RE still set; and slave deselection mid-frame.
Compare uninterrupted runs with stops/save-restores around those effects.

Searches found SSU notices for other Renesas families, including
[TN-H8*-A311A/E][other-erratum] and TN-SH7-A610A/E. Their listed targets and extra
registers differ; they do not establish H8/38606 errata. In particular, do not
import the H8SX notice's extra one-bit completion wait as a proven target rule.

[manual]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[registers]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=323
[flags]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=344
[arbitration]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=343
[phase]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=328
[pins]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=329
[initialization]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=338
[transmit]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=340
[receive]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=341
[receive-only]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=334
[pw-capture]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_main.c#L501-L533
[pw-accel]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_accel_bma150.c#L20-L65
[pw-eeprom]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_eeprom_m95512_bus.c#L7-L53
[pw-display]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_nt7508.c#L55-L64
[other-erratum]: https://www.renesas.com/en/document/tcu/usage-notes-ssu-h8sx1520-group-and-h8sx1582
