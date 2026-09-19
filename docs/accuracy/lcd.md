# NT7508 display controller

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

The controller implements serial command/data parsing, RAM and icon storage, addressing,
geometry, mapping, palettes, PWM/FRC, scan phase and inversion. Software reset, RESETB
and power save have different effects. The display API supplies interpreted pixels and
digital scan drive. Oscillator frequency/control settings affect the scan. The
datasheet and `pw` initialization/fill routines establish geometry and plane order.

## Limits and open questions

There is a known implementation gap in electrical controls. Converter/regulator/follower
enables, resistor ratio, booster, bias, temperature slope and contrast trim are stored
but do not affect exposed drive or the basic contrast value. The datasheet defines
effects for these controls. Firmware changing them can therefore produce an inaccurate
display result. Review the supplied voltage model and frontend information together;
the frontend should receive interpreted hardware output.

OTP control is also stored without a fusing model. Fusing requires an external programming
voltage whose delivery on this board is not established. Determine reachability before
adding a programming interface. External OSC1 selection stops internal advancement;
the board has no established external OSC1 driver.

RAM latch granularity, geometry changes during scan, some command/scan ties and restart
phase use explicit inferences. Pixel intensity expresses normalized programmed drive;
it is not a calibrated brightness curve. Glass persistence and optical appearance are
presentation work when a frontend requires them. Existing supply-control omissions
remain hardware questions regardless of that future presentation work.

## NT7508 behavior and sources

The table records the hardware rules. The [limits above](#limits-and-open-questions)
identify electrical effects the implementation still lacks. The command inventory
establishes decoding coverage; each command also needs its specified effect.
Novatek NT7508 V1.0, 2008-07-01, supplies the printed page references below.

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

For column wrapping, p. 15 contains a conflicting generic lock sentence, but the
command-specific description on p. 33 expressly wraps after the final column, consistent
with the complete map on p. 16. Retain wrap as the supported interpretation. Similarly,
use the explicit E2 reset list on p. 29 rather than expanding its scope from p. 45's
loose summary.

`pw` supplies useful independent checks. Its
[fill routine][pw-fill] sends light gray as `00 FF` and dark gray as `FF 00`:
the first byte is the high plane. Its [contrast setter][pw-contrast] invokes the [byte
sender][pw-send] separately for command and parameter; pending parameters must survive
CS. Its [initialization stream][pw-init] selects 64 rows starting at COM32, `0x95`
modulation, `F7=02`, `F6=0A`, and palette widths 0/5/7/9. Decoding that stream with
Novatek pp. 46–49 gives 3-frame FRC, 9-step PWM, a clock divisor of 3, and nominal frame rate
`122000 / (64 * 3 * 9) = 70.60185 Hz`. The factor 3 in this frame-rate
calculation is the clock divisor; the FRC sequence spans three such frames. These widths describe controller drive, not linear perceived brightness.
`FD`/`FE` in that stream are firmware script directives, not controller commands.

The pixel API averages each programmed palette entry's valid pulse widths over the
selected FRC frames and normalizes the result to 0..255. Retail's 0/5/7/9 widths
therefore produce 0/142/198/255. This view projects current RAM and geometry;
`display_drive` provides the scan timing and latched output. The controller voltage effects described above remain core work. A frontend can
apply a glass-response model to the resulting drive.

The controller retains scan phase, frame/FRC index, line counter, latched start line,
current output latch, inversion phase, and the clock/control projection. Settle the old
projection before a mutation, then apply the command at its actual completion time. A
RAM write must not retroactively rewrite the output already latched. The scan section
below resolves the latch granularity. Crop through board wiring after controller
mapping; centered COM32–95 happens to hide some common-direction mistakes, but that
coincidence is not the mapping rule. Keep icon enable independent of the selected RAM
page. Expose interpreted controller changes or requested pixels without making the
frontend decode commands.

Use arithmetic projection for stable scan spans. No global scheduler event per PWM edge
is needed. Preserve the state needed to resume a partially scanned frame, including when
firmware changes RAM or parameters mid-frame. The unresolved electrical controls and their settling behavior remain hardware work.
Frontend work can refine how the resulting drive is presented.

## NT7508 digital scan and command decoding

### Clock and modulation

Let `D = programmed_duty + icon_enabled`, `P` be PWM steps, and `F` be FRC frames. One
row consumes `P` PWM quanta; one frame consumes `D * P` quanta. These are controller
frames, independent of host refresh or firmware drawing. The command tables give the
following nominal timing. [Novatek pp. 46–49][nt-clock]

| Control | Decode |
| --- | --- |
| `F7`, parameter bit 0 | 0: internal oscillator; 1: external OSC1 input |
| `F7`, parameter bit 1 = 0 | Mode 0: frame rate `1008 / P` Hz; thus PWM quantum rate `1008 * D` Hz |
| `F7`, parameter bit 1 = 1 | Mode 1: quantum rate `Fosc / Q`; line rate `Fosc / (Q * P)`; frame rate `Fosc / (Q * P * D)` |
| `F6`, parameter bits 4:3 | `00`: 92,000 Hz; `01`: 122,000 Hz; `10`: 147,000 Hz; `11`: 184,000 Hz |
| `F6`, parameter bits 2:0 | `Q = FrameFQ + 1`, hence divisors 1–8; these settings apply to Mode 1 |
| `90`–`97`, bit 2 | 0: `F=4`; 1: `F=3` |
| `90`–`97`, bits 1:0 | `00` or `01`: `P=9`; `10`: `P=12`; `11`: `P=15` |
| `F3`, low four parameter bits | Signed two's-complement V0 contrast trim, −8 through +7; not oscillator trim |

Mode 0 therefore gives 112, 84, and 67.2 Hz for 9/12/15 PWM. Use the command-specific
Mode 0 table literally; do not apply the Mode 1 divider to it. The separate electrical
characterization gives 109/112/115 Hz minimum/typical/maximum at 128 duty, 9 PWM and
25 °C. The model uses the typical value at these conditions.
[Novatek pp. 48, 56–57][nt-rates]

For shade `s`, obtain its width from palette byte `2*s + frc/2`, low nibble for even
`frc`, high nibble for odd `frc`. FRC indices are 0–3 or 0–2; in 3-FRC mode the fourth
nibble is unused. Widths above `P` mean zero, not saturation. Reverse selects `s ^ 3`;
entire-on selects `3` and overrides reverse. In particular, entire-on still selects the
programmable black palette: p. 41's four-shade table specifies `11`, not an
unconditional full PWM pulse. Display-off takes priority over both. [Novatek pp. 18–19,
36, 41, 47][nt-overrides]

The `pw` initialization gives `D=64`, `P=9`, `F=3`, `Q=3`, `Fosc=122000`. Consequently a
quantum is `3/122000 s`, a row `27/122000 s`, and a frame `1728/122000 s`: approximately
24.590164 µs, 221.311475 µs, and 14.163934 ms. Its four shades have widths 0, 5, 7, and
9 in every used FRC frame. These settings determine the scan timing for this firmware.
[Initialization][pw-init], [fill byte order][pw-fill]

### Scan, latch, and polarity

The start-line register copies into the line counter at frame start; changing it
mid-frame does not change that frame's RAM-row sequence. CL advances the row counter.
Use `(latched_start + row) & 127`; an enabled icon occupies its own extra row and does
not scroll. The common start and scan direction map this sequence onto the 128 COM pins,
while ADC maps columns across all 128 SEG pins. Cropping comes after those mappings.
[Novatek pp. 15, 20, 23, 37–39, 44][nt-scan]

The p. 6 topology is RAM → FRC/PWM → display latch → SEG driver. The prose specifies a
128-bit latch and a CL-derived latch signal, but does not show the PWM subclock wiring.
A supported inference is a 128-bit binary latch refreshed once per PWM quantum. Evaluate
current RAM, overrides, the selected FRC palette and `quantum < width` at a quantum
boundary; hold those 128 bits until the next boundary. Align PWM to the leading edge of
the interval as the selected phase. A RAM write during a row can affect later quanta of
that row, while the current quantum retains its latched value.
[Novatek block diagram and timing prose, pp. 6, 15, 20][nt-block]

Frame inversion toggles `M` once per frame. In n-line mode, parameter zero selects frame
inversion and parameters 1–31 select `N=parameter+2` lines. Keep its divider continuous
across frames. Figure 8 explicitly shows `N=5` transitions at line 126, then next-frame
lines 3 and 8, without a transition at the intervening frame edge. The omitted middle of
that drawing is not parity-consistent with its first group; its local five-line cadence
is clear. The warning about even `D/N` causing DC bias also fits a free-running n-line
divider. `E4` returns to frame inversion. [Novatek pp. 21, 40–41][nt-inversion]

For a binary segment bit `b`, the normal-drive pin table is:

| `M` | Selected COM | Other COMs | SEG if `b=1` | SEG if `b=0` |
| --- | --- | --- | --- | --- |
| 0 | V0 | V4 | VSS | V3 |
| 1 | VSS | V1 | V0 | V2 |

Thus polarity changes electrical drive, not the stored shade. In power save, all COM/SEG
pins are VSS. These symbolic rails suffice for controller output; they do not require a
liquid-crystal appearance simulation. [Novatek p. 10][nt-pins]

### Compact progression and boundary choices

Store a time anchor and rational quantum remainder, quantum index within the row, row
index, FRC index, latched start line, current geometry, polarity and n-line remainder,
plus the 16-byte SEG latch. RAM and programmed registers remain their own state. For
stable settings, if `H` is time units per second and quantum rate is `a/b`, accumulate
`dt*a` over denominator `H*b`; integer division yields elapsed quanta and the exact
remaining fraction. Carry quanta through `P`, rows through `D`, and frames through `F`;
advance polarity by frame count or the continuous `N` divider. Preserve every remainder
in a save state. No floating-point rounding or host-call-dependent tick is needed.

Settle this old projection before every RAM/control mutation. If many quanta elapsed
without a mutation, compute the last quantum and its latch directly. There is no need to
visit the skipped edges. A consumer requesting drive over an interval can receive a
compact span with its starting counters, controls, and a borrowed RAM view while those
remain valid. Flush that span before mutating RAM; retaining only the latest RAM cannot
reconstruct a past frame. Do not copy 4 KiB per SPI byte merely to support an unused
output history.

The following narrow boundary rules complete the digital model where the manual supplies
steady-state behavior but no live-reprogramming waveform:

- Latch duty/icon and initial-COM geometry at frame start; latch PWM step
  count at row start and FRC length at frame start. Start-line's frame latch
  is explicitly documented; the other boundary choices keep complete scan
  slots when firmware reprograms the controller while it runs. Ordinary
  data/palette/reverse/entire changes feed the next quantum latch.
- Finish the current quantum under its old rate before an internal frequency
  change takes effect. Changing frequency must not reset row/FRC/polarity.
  A switch to external OSC1 stops internal clock advancement immediately;
  only actual OSC1 edges can advance that selection. The manufacturer's
  internal-clock connection leaves OSC1 open. The board investigation and
  `pw` identify no external LCD-clock driver, so the board model leaves OSC1
  undriven.
- Treat scan edges at a command's timestamp as preceding that completed SPI
  byte. On that tie the old value is captured, and the next eligible latch
  sees the write. This ordering must be independent of host chunk sizes.
- `AB` starts an oscillator stopped by reset. Repeated `AB` does not restart
  an already-running scan. Display `AE`/`AF` gates visible drive while scan
  continues. `A9` stops scan and forces VSS while retaining RAM/registers;
  `A8`/`E1` releases it, restoring the previous oscillator-enable setting.
  Start a fresh row-zero/FRC-zero phase when the oscillator starts or a
  running oscillator is released from power save, loading current
  geometry/start line. A repeated release while already awake is inert.
  This restart phase is an inferred model parameter.
- Change n-line mode at the next row boundary, preserving `M` and restarting
  its line divider. `E4` preserves `M` until the next frame boundary, then
  resumes frame-based toggling; it does not restart the frame or FRC index.
- RESETB resets the p. 29 control list and stops the oscillator, but is not a
  RAM-clear command. `E2` resets only the shorter p. 29 list: cursor/RMW,
  start-line register, resistor ratio, contrast, DDL, palettes, and FRC/PWM.
  It preserves RAM, power circuits, display enable, geometry, oscillator
  selection, and scan progression, apart from the changed settings taking
  effect at their boundaries. Actual supply loss terminates powered scan;
  cold RAM initialization is distinct from reset and power-save retention.

Sources for these distinctions: [Novatek pp. 8, 29, 36, 44–45, 53][nt-reset],
[board investigation][nt-board], [firmware serial wiring][pw-wiring]. The
boundary choices are centralized physical-model rules, not accuracy modes.

### Complete assigned command inventory

This inventory is for a command byte when no parameter is pending. A pending parameter
consumes the next command-mode byte; CS alone does not cancel ordinary two-byte
commands. The board uses four-wire SPI. [Novatek instruction tables, pp.
30–32][nt-instructions], [pw byte sender][pw-send]

| Byte(s) | Operation; parameter when present |
| --- | --- |
| `00`–`0F`, `10`–`17` | Low/high seven-bit column address; both clear byte-plane bit Y0 |
| `20`–`27`, `28`–`2F` | Regulator resistor ratio; converter/regulator/follower enables |
| `40`–`43`, `44`–`47` | Start-line; initial COM; each takes a low-seven-bit parameter |
| `48`–`4B`, `4C`–`4F` | Duty parameter 16–128, other values ignored; inversion low-five-bit parameter |
| `50`–`57` | Bias 1/5 through 1/12 |
| `64`–`67`, `6C`–`6F` | DC-DC ratios 3/4/5/6 and 7/8/8/8; `68`–`6B` do not match the documented bit encoding |
| `81` | Contrast, low-six-bit parameter |
| `88`–`8F` | Two FRC palette nibbles, one parameter byte; shade/pair from command low bits |
| `90`–`97` | FRC/PWM selection, including the duplicate 9-PWM encodings |
| `A0`/`A1`, `A2`/`A3` | SEG direction; icon disable / enable-and-select-icon-page |
| `A4`/`A5`, `A6`/`A7` | Entire-display override; reverse display |
| `A8`/`A9`, `AB`, `AE`/`AF` | Normal/power save; oscillator start; display off/on |
| `B0`–`BF`, `C0`–`CF` | Main RAM page; COM direction with bits 2:0 ignored |
| `E0`, `EE` | Enter modify-read/save column; exit/restore saved column |
| `E1`, `E2`, `E3`, `E4` | Release power save; software reset; NOP; release n-line inversion |
| `E8` | Three-wire-only DDL, parameter + 1 bytes; four-wire mode does not enter DDL |
| `F1`, `F3`, `F4`, `F6`, `F7` | Temperature coefficient, signed V0 trim, OTP control, frequency, clock source/mode; one parameter each |

`A2` disables icon output without selecting a main page; `B0`–`BF` select a main page
without disabling icon output. `A3` does both documented actions. Decode the assigned
`F1/F3/F4/F6/F7` commands before the broad `F0`–`FF` factory-test range. F4 modes are
normal `00`, trim test `01`, fusing `11`; `10` is unassigned. Fusing requires roughly 7
V on OTP_PWR for two seconds; ordinary board voltage and an F4 command cannot burn a new
persistent trim. Keep volatile test selection separate from fused trim. [Novatek pp. 7,
34, 46, 49–51][nt-clock]

Only `E3` is expressly documented as NOP. No primary source says every unassigned byte
has that encoding. For a total decoder, use inert one-byte behavior for
unassigned/factory-test bytes as the selected model, preserving ordinary serial progress
rather than stopping the emulator. Similarly ignore E8 on the four-wire board without
swallowing the following byte. Record the inert choice here once; do not manufacture
errors for firmware or claim it is a measured test-mode implementation.

## Implementation and checks

The [controller](../../crates/hs-core/src/devices/nt7508.rs) and
[scan model](../../crates/hs-core/src/devices/nt7508/scan.rs) own parsing and timed drive.
Hachiware's [LCD cases](https://github.com/lumirth/hachiware/blob/main/cases/lcd.py)
check command effects exposed by the adapter; tests alongside the implementation cover
mapping and scan rules. [Retail verification](../TESTING.md#private-firmware-regression-versus-smoke)
compares complete exported images and histories with a reviewed software baseline.
A parser/readback assertion or an unchanged image cannot establish the missing supply,
bias or trim effects. Those need behavior that changes with the relevant control.

[nt-serial]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=12
[nt-ram]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=15
[nt-duty]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=39
[nt-map]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=44
[nt-overrides]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=41
[nt-icon]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=34
[nt-reset]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=29
[nt-pwm]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=46
[nt-clock]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=48
[nt-rates]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=56
[nt-scan]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=20
[nt-block]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=6
[nt-inversion]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=21
[nt-pins]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=10
[nt-instructions]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=30
[nt-board]: https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/doc/Board.md#L12-L33
[pw-send]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_nt7508.c#L53-L64
[pw-contrast]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_nt7508.c#L133-L145
[pw-fill]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_nt7508.c#L241-L315
[pw-init]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_nt7508.c#L968-L983
[pw-wiring]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_ssu_init.c#L4-L18
