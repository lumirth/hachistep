# NT7508 controller and M95512 power transitions

Research scope: the current `nt7508.rs`, `m95512.rs`, and machine power/reset
owners. This note addresses controller behavior and persistent cells; it does
not require a calibrated model of LCD appearance. Sources are Novatek
**NT7508 V1.0, 2008-07-01**, ST **DS4192 Rev 24**, and `lumirth/pw` at
**6dc7bc09950078fa3fe0dffa4dae34e9549a99da**. Page references are printed pages.

## NT7508: corrections and source map

| Behavior | Required rule | Novatek reference |
| --- | --- | --- |
| Interface | Serial is write-only. CS resets partial-byte assembly. Three-wire DDL is `parameter + 1`; it is a pin-selected interface. | [pp. 8, 11–13, 46][nt-serial] |
| Addressing | Column commands reset plane phase; byte address wraps. Icon data uses DB0 only. | [pp. 15–16, 33][nt-ram] |
| Scan | Start line loads at frame start; CL advances the line counter and display latch. | [pp. 15, 20][nt-ram] |
| Partial display | Duty is literal 16–128; invalid values do nothing; reset duty is 128. | [pp. 29, 39][nt-duty] |
| Mapping | SEG and COM reversal operates across 128 outputs. | [p. 44][nt-map] |
| Overrides | Display-off wins; entire-on overrides reverse. | [pp. 36, 41][nt-overrides] |
| Icon | A3 enables icon output and selects its page; page selection is a separate operation. | [p. 34][nt-icon] |
| Reset/power | E2 preserves RAM and has fewer reset effects than RESETB. A9 stops oscillator/supply and drives COM/SEG to VSS. | [pp. 29, 45][nt-reset] |
| Modulation | FRC: 3/4 frames; PWM: 9/12/15 steps. Palette nibbles exceeding PWM count produce zero. | [pp. 46–47][nt-pwm] |
| Trim | F3 uses signed low four bits. OTP fusing additionally requires external programming voltage. | [pp. 49–51][nt-clock] |

The current renderer reverses the two gray planes, flips within the 96-column
viewport, and applies reverse after entire-on. Its duty defaults to 127 and
accepts invalid values. One `icon` flag mixes display enable with the write
cursor. Those are concrete fixes before adding further presentation work.
The current oscillator, palettes, frequency, duty, and inversion controls are
largely stored without affecting scan progression.

The apparent wrap ambiguity can be resolved now: p. 15 contains a conflicting
generic lock sentence, but the command-specific description on p. 33 expressly
wraps after the final column, consistent with the complete map on p. 16. Retain
wrap as the supported interpretation. Similarly, use the explicit E2 reset
list on p. 29 rather than expanding its scope from p. 45's loose summary.

`pw` supplies useful independent checks. Its
[fill routine][pw-fill] sends light gray as `00 FF` and dark gray as `FF 00`:
the first byte is the high plane. Its [contrast setter][pw-contrast] invokes
the [byte sender][pw-send] separately for command and parameter; pending
parameters must survive CS. Its [initialization stream][pw-init] selects
64 rows starting at COM32, `0x95` modulation, `F7=02`, `F6=0A`, and palette
widths 0/5/7/9. Decoding that stream with Novatek pp. 46–49 gives 3-frame FRC,
9-step PWM, and nominal `122000 / (64 * 3 * 9) = 70.60185 Hz`.
These widths describe controller drive, not linear perceived brightness.
`FD`/`FE` in that stream are firmware script directives, not controller commands.

Recommended representation: retain scan phase, frame/FRC index, line counter,
latched start line, current row data, inversion phase, and the clock/control
projection. Settle the old projection before a mutation, then apply the command
at its actual completion time. A RAM write must not retroactively rewrite the
row already latched for output. Crop through board wiring after controller
mapping; centered COM32–95 happens to hide some common-direction mistakes, but
that coincidence is not the mapping rule. Keep icon enable independent of the
selected RAM page. Expose interpreted controller changes or requested pixels
without making the frontend decode commands.

Use arithmetic projection for stable scan spans. No global scheduler event per
PWM edge is needed. Preserve the state needed to resume a partially scanned
frame, including when firmware changes RAM or parameters mid-frame. Detailed
analog settling can follow the first frontend; controller phase cannot.

## M95512: concrete protocol rules

| Behavior | Rule | ST reference |
| --- | --- | --- |
| SPI | Input on rising edges; output on falling edges; deselection floats Q. | [§§3–4, pp. 5–6][ee] |
| WEL | Clear at WRITE/WRSR completion, not start. POR and completed WRDI also clear it. | [§6.3.2, p. 14][ee-status] |
| WRSR | Exactly one status byte; deselect before the next rising edge. Persistent mask `0x8C`; reserved bits read zero. | [§6.4, pp. 15–16][ee-wrsr] |
| Protection | BP protects upper quarter/half/all; SRWD plus low W protects status writes. | [§§5.5, 6.4][ee-wrsr] |
| Page writes | Complete-byte CS rise starts one internal operation. Wrap inside 128 bytes; repeated addresses take the last byte. | [§6.6, p. 18][ee-write] |
| Cells | Erase addressed bytes, then program them. **Erased reads 0; programmed reads 1.** | [§6.6, p. 18][ee-write] |
| Power | POR resets volatile write state; valid supply must persist through programming. | [§5.1, p. 8][ee-power] |

Current changes needed: move WEL clearing to completion; reject an overlong
WRSR instead of accepting its last byte; replace busy-power-off rejection with
cell progression. The existing page-wrap mask and array-read wrap are useful
to retain. Completion of an internal write must remain independent of CS,
CPU sleep, and MCU-only reset.

RDSR supports continuous polling while busy. Preserve refresh between output
bytes; the exact internal capture instant relative to a coincident completion
is not established by the timing diagram. Do not turn the current early
`transmit(status())` call into a claimed physical latch point. HOLD is a serial
pause, not cancellation of internal programming, but add a board route only
if that pin is actually connected to a controllable signal. Do not import
identification-page commands from another M95512 variant.

## A compact default for interrupted programming

The documented two-stage operation gives a substantially better starting model
than either atomic replacement or refusing power loss. ST does not specify
the phase-duration split or the order in which marginal cells cross their
read thresholds. The following is an implementable inference, not a measured
bit-order claim:

1. Before the terminating CS edge, data exists only in the volatile page
   buffer. Power loss discards it without changing cells.
2. At acceptance, freeze the addressed-byte mask, original values, targets,
   start time, duration, and physical-part parameters. Maintain WIP and WEL
   through programming. Treat a page as one operation over its selected cells,
   not 128 serial byte operations.
3. During erase, an affected old byte `O` becomes `O & ~E(t)`, where `E` is a
   monotonic mask of erased cells. Once erase ends, programming proceeds from
   zero as `T & P(t)`, where `T` is the target and `P` is a monotonic completion
   mask. Unaddressed bytes remain untouched.
4. Use a deterministic threshold schedule associated with the physical part,
   address, and bit. A half-duration split and a stable distribution of cell
   thresholds within each phase are a compact initial choice; neither is a
   measured ST timing parameter. Keep those choices centralized so evidence
   can refine them without replacing the model. Do not base them on host call
   count or consume new randomness when loading a save state.
5. On supply collapse, evaluate the partial cells, report the resulting
   persistent changes, and cancel the internal operation. On power-up, reload
   those cells and reset volatile protocol state. Successful completion always
   produces the entire requested value. A same-value write still performs an
   erase/program cycle and can be damaged by interruption.

Apply the same inferred mechanism to the writable nonvolatile status cells,
restricted to `0x8C`; leave the array alone during WRSR. Normal status-register
visibility changes at completion; after interrupted programming, cold start
loads the resulting persistent status. Keep the interruption calculation in
the device owner so machine-level power handling cannot accidentally bypass it.

This needs no per-cell scheduler events or floating-point simulation. Store
the operation descriptor and compute cell thresholds only when an observation
requires them, especially power loss and completion. Snapshotting preserves
the descriptor, physical parameters, and time; EEPROM-save extraction must
explicitly settle/project the persistent cells it represents. A supply value
that disables the chip must reach this transition, rather than merely updating
a machine condition field.

The firmware makes this behavior useful. Its [page writer][pw-page] polls WIP,
issues WREN, transmits 128 bytes, raises CS, and returns. Its
[mirrored writer][pw-mirror] writes payload/checksum pairs separately.
[WalkStartCommit][pw-commit] sets a recovery marker, copies pages, clears the
marker, then performs more updates; [BootRestore][pw-restore] handles that
marker. Preserve those individual operations and let actual firmware perform
recovery.

Targeted behavioral checks: power removal before CS, during erase, between
phases, during programming, and after completion; the same cases for WRSR;
WEL while busy; overlong WRSR; MCU reset during programming; LCD plane order,
full-width reversal, entire-on plus reverse, partial-duty boundaries, and a
RAM/start-line change straddling a scan latch. Repeat relevant cases with
different host execution chunk sizes and save-state restoration.

[nt-serial]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=12
[nt-ram]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=15
[nt-duty]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=39
[nt-map]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=44
[nt-overrides]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=41
[nt-icon]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=34
[nt-reset]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=29
[nt-pwm]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=46
[nt-clock]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=48
[ee]: https://www.st.com/resource/en/datasheet/m95512-r.pdf#page=5
[ee-status]: https://www.st.com/resource/en/datasheet/m95512-r.pdf#page=14
[ee-wrsr]: https://www.st.com/resource/en/datasheet/m95512-r.pdf#page=15
[ee-write]: https://www.st.com/resource/en/datasheet/m95512-r.pdf#page=18
[ee-power]: https://www.st.com/resource/en/datasheet/m95512-r.pdf#page=8
[pw-send]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_nt7508.c#L53-L64
[pw-contrast]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_nt7508.c#L133-L145
[pw-fill]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_nt7508.c#L241-L315
[pw-init]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_nt7508.c#L968-L983
[pw-page]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_eeprom_m95512_io.c#L507-L574
[pw-mirror]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_eeprom_m95512_io.c#L18-L35
[pw-commit]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L320-L336
[pw-restore]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_storage.c#L119-L145
