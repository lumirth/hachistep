# ADC and battery sensing

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

The ADC implements channel selection, sample/hold, conversion work, result alignment,
completion requests, cancellation and external triggering. AVCC supplies its reference.
The nominal transfer rounds at half-LSB boundaries and clips to the ten-bit range.
The manual and subclock application note support conversion lengths and power behavior.

## Limits and open questions

The sampling interval, or acquisition aperture, occupies four of 31 converter steps
in the selected model.
Live channel/clock changes, premature conversion after module enable and a disconnected
input use the retained capacitor and conversion state. These choices affect fast analog
changes and sequences outside the prescribed initialization procedure.

The board's battery circuit is less well established than the ADC transfer. `pw`
switches P84, samples PB3/AN3 and compares the result against EEPROM calibration.
HachiStep models that path as battery voltage minus a nominal 600 mV drop. Firmware
establishes the sequence and response direction, but not that circuit or drop.
Consequently a predicted battery-warning voltage has weaker support than the ADC code
for an explicitly supplied pin voltage. Component identification and existing board
evidence may refine this without changing the ADC itself.

## Sources and applicability

The H8/38606 target addition leaves these peripherals unchanged. The applicable baseline
is REJ09B0152-0300 rev. 3.00. Page references are printed manual pages; PDF pages are 34
higher. ([Target applicability][target], [manual][manual])

## Conversion, triggering and power modes

AMR selects conversion lengths: CKS `00/01/10/11` means at most `124φ / 62φ / 31φ /
31φW` states. Channels 4–9 select AN0–AN5; other codes disconnect the input mux. ADSF
zero aborts, one starts; successful completion commits ADRR bits 15–6, clears ADSF and
sets IRRAD together. Changing channel or conversion speed is prescribed with ADSF
cleared. ([Manual §§17.3–17.4][adc])

ADRR survives MCU reset; AMR and ADSR reset. Sleep preserves conversion;
watch/standby/module standby halt and retain it. Subactive/subsleep permit watch-source
conversion, with the documented restriction that CPU φSUB must equal φW. Do not turn a
program violating that restriction into a core exception; retain the source-domain
conversion as the nominal inference. ([Manual table 17.2, p. 354][adc-power])

Conversion state consists of a phase in 31 half ADC-clock steps, a held analog sample
and a pending result. Each step spans `4φ, 2φ, φ, φW` for the four CKS values,
reproducing all four totals. This makes an active CKS change consume the remaining
converter work on the newly selected source, rather than restarting 31/62/124 states.
Keep the held sample unchanged after capture. Before capture, a channel write affects
the mux normally. For a disconnected mux, retain the previous sample-capacitor value and
complete normally. These live-reconfiguration outcomes are inferences consistent with
the shared sample-and-hold/SAR structure.

The selected acquisition aperture is four of the 31 converter steps. The manual does not
specify this placement; expressing it in converter steps preserves the sample phase
across speed selections. Retain readiness separately: §17.7.3 requires 10φ cycles after
leaving module standby before software starts conversion. This is an analog-settling
requirement, not permission to insert an automatic CPU wait. For premature starts,
continuing conversion with the previously held sample if acquisition precedes readiness
is the selected inference. ([Manual pp. 359–360][adc-settling])

Implement external trigger through the physical TEST/ADTRG input: PMRB bit 3 selects
ADTRG, AMR.TRGE enables it, IEGR bit 5 selects falling (`0`) or rising (`1`).
Synchronize detection with the operating clock and enter the same ADSF start path. A
trigger while already converting need not queue/restart a conversion; ignoring it is the
natural level-ADSF inference. Do not replace this with a host command that writes a
conversion result. ([Manual §3.4.3, §8.5.2, §17.4.2, figure 17.2][adc-trigger])

Trigger detection uses two sampled stages. A coincident completion clears ADSF after
trigger detection; a trigger seen while ADSF is set does not restart the conversion.

The [transfer function](#reference-and-quantization) below determines the held sample's
result. AVCC follows the board supply unless an analog-supply fixture overrides it.

## Reference and quantization

The ADC's upper reference is AVCC, package pin 1, and its lower reference is VSS. AVCC
is an external analog supply input, distinct from digital VCC; there is no selectable
internal fixed 3.3 V ADC reference. VCref is the comparator reference input and does not
feed the ADC. See REJ09B0152-0300 §17.1/Fig. 17.1, printed p. 349; §17.2/Table 17.1, p.
350; §1.4/Table 1.1, p. 4; comparator §§18.1–18.3, pp. 361–363. The ADC electrical table
rates AVCC from 1.8 to 3.6 V. ([ADC][board-adc], [board-pins][board-pins], [board-comparators][board-comparators],
[board-electrical][board-electrical], [target addition][board-target])

Figure 17.6, printed p. 358, depicts midpoint code transitions: its illustrative
three-bit converter changes 0→1 at half an LSB and 1→2 at 1.5 LSB. Section 17.6, p. 357,
and Table 21.7, p. 405, specify ±0.5 LSB quantization error. Therefore the nominal
ten-bit transfer is:

```text
LSB = AVCC / 1024
code = clamp(floor(1024 * Vin / AVCC + 1/2), 0, 1023)
ADRR = code << 6
```

For nonnegative integer millivolts and positive AVCC, use `min(1023, (2048 * Vin + AVCC)
/ (2 * AVCC))` with sufficiently wide arithmetic. Half rail gives 512; the highest code
begins at 1022.5 LSB. Multiplying by 1023 is not the ideal transfer. Exact midpoint ties
go upward in this chosen deterministic realization. ([Accuracy definitions/Fig.
17.6][board-accuracy])

## What the board and firmware establish

`BatterySample` drives P84 high as an output, delays, averages eight channel-7 (PB3/AN3)
conversions, then drives P84 low and releases it to input. The ADC result is divided by
64 to remove alignment, not to convert into volts. `BatteryCheckLow` declares low
battery when that count is at or below the EEPROM calibration payload times
`scaleFactor/20`; routine checks use 20 and the startup loop uses 19. Thus the measured
count must generally increase with battery voltage. ([Battery routines, lines
47–119][board-battery],
[startup constant and loop][board-startup])

`FactoryBatteryCalibrate` stores the measured raw count, with a nibble checksum, in the
mirrored EEPROM records. It supplies neither a universal voltage constant nor a divider
ratio. The manufacturer's fixture voltage is not encoded in that routine. Never infer
the emulator's analog voltage from the loaded EEPROM threshold. ([Calibration, lines
198–226][board-calibration])

The original NTR-PHC-01 photographs show D1, a three-terminal component near R21/R22 and
C22/C23, plus unidentified U2/U3/U4/U8. They establish available components but not D1's
internal connection, forward drop, or continuity to the ADC. The public board
investigation does not identify an ADC reference rail. Its statement that a CR2032 is
nominally 3.3 V is not a reference measurement. The inspected public full EEPROM dump
contains `00 00 01` at both calibration records, so it supplies no useful calibration
voltage/count pair. ([Original side-A photograph][board-photo-a], [side-B
photograph][board-photo-b],
[board investigation][board-board], [public dump][board-dump])

## Selected nominal circuit and defaults

Use a battery-fed AVCC and an enabled sense path with one effective forward voltage
drop. This is the smallest useful circuit consistent with the firmware's monotonicity
and its switched measurement sequence. It does not identify D1 as that path or assert
that exactly one physical junction is present.

```text
board supply ─────────────────────────── AVCC
P84 high drive ── effective junction ─── PB3/AN3
                                         │
                                  off-state discharge
                                         │
                                        VSS
```

| Physical quantity | Selected nominal value or rule |
| --- | --- |
| Board rail | Existing `supply_millivolts`, default 3000 mV |
| AVCC | Board rail; follows supply changes |
| Effective sense drop | 600 mV, a tunable board constant without temperature dependence |
| Enabled PB3 voltage | `supply.saturating_sub(sense_drop)` |
| Sense enable | P84 configured as output and driven high |
| P84 low or high impedance | PB3 becomes zero immediately in the selected static network |
| Additional board RC settling | No additional board delay; ADC aperture and module-wake readiness remain separate |
| Whole-board power off / zero rail | No conversion progression or division by zero; external voltages cannot back-power this nominal board |

600 mV represents an ordinary forward-biased silicon-junction-scale drop. It is a chosen
starting value, not a measured Pokéwalker value. Keep it independent of firmware, EEPROM
contents, and the low-battery decision. A later physical fit can replace this one
constant or add the measured temperature/settling dependence without changing the ADC
quantizer.

At 3300/3000/2700/2400 mV supply this model produces PB3 voltages 2700/2400/2100/1800 mV
and ideal counts 838/819/796/768. A direct analog-pin fixture overrides PB3 voltage
after this network calculation, while conversion still uses AVCC. A switch or supply
change after the aperture must not alter the ADC's already-held sample. A stopped MCU
does not by itself change P84's drive; ordinary pin/reset/power resolution owns that
change.

`avcc_override_millivolts` supplies an external analog-supply fixture. Timer W driving
P84 high also enables the sense path; an input pull-up alone does not.

## Alternatives and evidence needed to refine the circuit

- A plain resistor divider with AVCC on the same battery gives a constant
  count and cannot implement the observed low-battery comparison.
- A fixed voltage at PB3 with battery-fed AVCC makes counts rise as the battery
  falls, opposite to the comparison in the matching firmware.
- A regulated AVCC with a divided battery input gives the correct direction
  and remains a plausible alternative. No reference-regulator identity, output
  voltage, divider ratio, or relevant net continuity was established here;
  adopting 3.3 V as that physical rail would be unsupported.
- A battery-relative input with one or several junction drops also gives the
  correct direction. The selected effective-drop model needs fewer unsupported
  circuit elements; photo component count alone cannot determine its numeric
  drop or distinguish it from a regulated-reference network.

Useful calibration evidence is a small set of simultaneous battery, AVCC, PB3 and
ADC-count observations during P84-high measurement, plus a P84-low observation. If AVCC
tracks the battery and `battery − PB3` is roughly constant, fit the drop directly. If
AVCC is regulated and PB3 follows a ratio, use those measured rails and divider instead.
Record temperature and sampling delay so device variation and settling are not silently
absorbed into a universal constant. A factory threshold word alone cannot resolve these
alternatives.

## Firmware acquisition sequence

[`BatterySample`](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_battery.c#L55-L87)
wakes the ADC, executes five NOPs, selects AN3 and CKS 10 or 11, polls ADSF, gates the
module off, then reads retained ADRR. Five two-state NOPs match the documented 10φ
settling requirement. This sequence corroborates acquisition and retention independently
of the selected battery circuit.

## Implementation and checks

The [ADC](../../crates/hs-core/src/mcu/adc.rs) owns acquisition and conversion;
[board routing](../../crates/hs-core/src/machine.rs) supplies voltages.
Hachiware's [ADC cases](https://github.com/lumirth/hachiware/blob/main/cases/adc.py)
check clock selection, trigger routing, disconnected input and selected battery-supply
responses. The battery cases verify the nominal circuit's arithmetic, not its physical
identification. [Clock-obligation tests](../../crates/hs-core/tests/clock_obligations.rs)
and [machine tests](../../crates/hs-core/tests/kernel.rs) cover held samples, gating,
AVCC midpoint transitions and restoration.

The [manufacturer leads](../SOURCES.md#located-manufacturer-material) include a joint
comparator/ADC application. It can strengthen peripheral interaction reasoning without
establishing this board's battery circuit.

[manual]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[target]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=2
[adc]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=385
[adc-power]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=388
[adc-settling]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=393
[adc-trigger]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=387
[board-adc]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=383
[board-pins]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=38
[board-comparators]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=395
[board-electrical]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=439
[board-accuracy]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=392
[board-target]: https://www.renesas.com/en/document/tcu/addition-h838606-group
[board-battery]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_battery.c#L47-L119
[board-startup]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L59-L231
[board-calibration]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_selftest.c#L198-L226
[board-photo-a]: https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/pics/sidea-bare-02.jpg
[board-photo-b]: https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/pics/sideb-bare-01.jpg
[board-board]: https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/doc/Board.md
[board-dump]: https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/dumps/bin/64k-full-rom.bin
