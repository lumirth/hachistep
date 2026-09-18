# ADC reference and nominal battery-sensing circuit

The chip reference and quantizer are established; the external battery-sensing network
below is a deliberately chosen nominal circuit, not a recovered board netlist. The
H8/38606 addition does not replace the family ADC. The inspected firmware is public
`lumirth/pw` revision `6dc7bc09950078fa3fe0dffa4dae34e9549a99da`.

## Established chip behavior

The ADC's upper reference is AVCC, package pin 1, and its lower reference is VSS. AVCC
is an external analog supply input, distinct from digital VCC; there is no selectable
internal fixed 3.3 V ADC reference. VCref is the comparator reference input and does not
feed the ADC. See REJ09B0152-0300 §17.1/Fig. 17.1, printed p. 349; §17.2/Table 17.1, p.
350; §1.4/Table 1.1, p. 4; comparator §§18.1–18.3, pp. 361–363. The ADC electrical table
rates AVCC from 1.8 to 3.6 V. ([ADC][adc], [pins][pins], [comparators][comparators],
[electrical][electrical], [target addition][target])

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
17.6][accuracy])

## What the board and firmware establish

`BatterySample` drives P84 high as an output, delays, averages eight channel-7 (PB3/AN3)
conversions, then drives P84 low and releases it to input. The ADC result is divided by
64 to remove alignment, not to convert into volts. `BatteryCheckLow` declares low
battery when that count is at or below the EEPROM calibration payload times
`scaleFactor/20`; routine checks use 20 and the startup loop uses 19. Thus the measured
count must generally increase with battery voltage. ([Battery routines, lines
47–119][battery],
[startup constant and loop][startup])

`FactoryBatteryCalibrate` stores the measured raw count, with a nibble checksum, in the
mirrored EEPROM records. It supplies neither a universal voltage constant nor a divider
ratio. The manufacturer's fixture voltage is not encoded in that routine. Never infer
the emulator's analog voltage from the loaded EEPROM threshold. ([Calibration, lines
198–226][calibration])

The original NTR-PHC-01 photographs show D1, a three-terminal component near R21/R22 and
C22/C23, plus unidentified U2/U3/U4/U8. They establish available components but not D1's
internal connection, forward drop, or continuity to the ADC. The public board
investigation does not identify an ADC reference rail. Its statement that a CR2032 is
nominally 3.3 V is not a reference measurement. The inspected public full EEPROM dump
contains `00 00 01` at both calibration records, so it supplies no useful calibration
voltage/count pair. ([Original side-A photograph][photo-a], [side-B
photograph][photo-b],
[board investigation][board], [public dump][dump])

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
| Effective sense drop | 600 mV, a tunable board constant, initially temperature-independent |
| Enabled PB3 voltage | `supply.saturating_sub(sense_drop)` |
| Sense enable | P84 configured as output and driven high |
| P84 low or high impedance | PB3 tends to zero; use zero immediately in this first static network |
| Additional board RC settling | No additional delay initially; retain the ADC's separate aperture and module-wake readiness |
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

[adc]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=383
[pins]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=38
[comparators]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=395
[electrical]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=439
[accuracy]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=392
[target]: https://www.renesas.com/en/document/tcu/addition-h838606-group
[battery]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_battery.c#L47-L119
[startup]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L59-L231
[calibration]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_selftest.c#L198-L226
[photo-a]: https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/pics/sidea-bare-02.jpg
[photo-b]: https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/pics/sideb-bare-01.jpg
[board]: https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/doc/Board.md
[dump]: https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/dumps/bin/64k-full-rom.bin
