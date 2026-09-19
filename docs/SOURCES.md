# Sources and provenance

## Supplied images

Firmware was extracted from the user's File Library archive `pw-inputs.zip`:

```
Archive bytes: 248796246
Archive SHA-256: e6400eee35504fea477ae466ed33bcc8c5b5cd313962ad56c351e7c24e1c3460
Member: pw-inputs/firmware/nintendo/pokewalker/retail-48k/pokewalker.bin
ROM bytes: 49152
ROM SHA-256: f9e210a3b74afbbd12c5a66a51cc05cb9fbac986805ff0a3bfb4be6074d15607
Reset vector: 0x02c4
```

The archive was inspected for a 64 KiB EEPROM and did not contain one. The EEPROM used
in the runs was acquired separately from the user's File Library:

```
EEPROM bytes: 65536
EEPROM SHA-256: 9b9d7ac29b3d27de8fed1aca392c91ec559a53c2c22f2a9c860c980895539008
```

The images are unmodified. Retail verification checks their identities before and after
execution against `workloads/retail.json`.

## Primary hardware references

- Hitachi H8/300H programming manual ADE-602-053A from the supplied archive;
  related Renesas H8/300H software manual:
  https://www.renesas.com/en/document/mas/h8300h-series-software-manual
- Renesas H8/38602R hardware manual, with target-specific addition:
  https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
  https://www.renesas.com/en/document/tcu/addition-h838606-group
- Bosch BMA150 manufacturer datasheet:
  https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf
- ST M95512 family datasheet; apply only the installed variant's features:
  https://www.st.com/resource/en/datasheet/m95512-w.pdf
- Novatek NT7508 v1.0:
  https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf

The archive also carries an SSD1854 datasheet. Its presence is not evidence that the LCD
owner should implement that controller. HachiStep implements NT7508 commands and the
reached `pw` driver behavior. The original PDF bytes are not redistributed. These hashes
identify the supplied copies.

| Document | SHA-256 |
| --- | --- |
| `components_bma150_datasheet.pdf` | `8e07fb86bce3daaa2c4b9b86558dfcba8ae6c861e4e1696cd8006b022d514eff` |
| `architecture_h8-300h_programming-manual-ade-602-053a.pdf` | `5c79702dcafcba0adacf77b8672bb1358ad5519ebf20d98e1b2f88000a32ed98` |
| `devices_h8-38602r_h8-38606-addition-note.pdf` | `416cd2b732a060b038519b0e38e935bf675a2fb99cd2d1554bc0daaf732ed2bb` |
| `components_ssd1854_datasheet.pdf` | `27a6bf6140ec2ae72898ed1e91526a88592dc6d9c9b2165ee019cc6c99782ff9` |
| `components_m95512r_datasheet.pdf` | `0f91e5ebc32c8eed6f389be81b01a2e90aa94c3a9b118a5fac428cd61af76bab` |
| `devices_h8-38602r_hardware-manual.pdf` | `5029638c5ab3e3448fbadb9dcbe689ff8e74fbd50412e3abd5720f1651a1cc0f` |

## Decompilation evidence

`lumirth/pw` was read at commit `6dc7bc09950078fa3fe0dffa4dae34e9549a99da`:
https://github.com/lumirth/pw/tree/6dc7bc09950078fa3fe0dffa4dae34e9549a99da

Relevant files include `src/application/pw_accel_bma150.c`, `pw_nt7508.c`,
`pw_eeprom_m95512_bus.c`, `pw_eeprom_m95512_io.c`, `pw_battery.c`,
`pw_player_input.c`, `pw_power.c`, `pw_rtc.c`,
`src/support/lib_common.c`, serial/IR setup, and startup/register headers. These
establish reached accesses and software intent. Read the executed sequences when source
comments differ from the hardware evidence.

Examples of concrete integration evidence are active-high button sampling, separate LCD
command/parameter selected intervals, the BMA protected-window initialization,
clock/module setup around SLEEP, battery sampling/polling, watchdog service and RTC
stable reads. The owner notes below distinguish manufacturer requirements from selected
physical parameters and circuit inferences.

## Implementation and diagnostic provenance

The independent fixture corpus is maintained in
[hachiware](https://github.com/lumirth/hachiware), without a dependency on
`hs-core`. Its expectations record their evidence. The HachiStep adapter exports
observations without owning those expectations.

## Manual sections

Printed page numbers differ from PDF indices. Use the named sections and diagrams when
comparing the manuals with the implementation.

- REJ09B0152-0300 §20.1, printed pp.372–375: register physical access widths and
  state counts. `hachiware/spec/register_access.tsv` is a separate 95-row
  transcription, not generated from production routing.
- §3.8.5: LDC/ANDC/ORC/XORC following-instruction interrupt deferral; RTE is not
  in that list. §3.8.6: EEPMOV.B versus EEPMOV.W NMI acceptance and saved next PC.
- §§3.4.1/3.5.1 and IEGR: NMI priority/latch and edge selection; §6.3 Table 6.1
  distinguishes low-at-reset bootstrap/debug straps from ordinary user mode.
- §10, particularly input/output timing and §10.7 conflict notes: Timer W
  compare/capture/buffer semantics and external clock. The implementation uses
  input pipelines resolved at reference-clock edges.
- §13: AEC/PWM, CUE/CRC, independent/cascaded counters, clock and IRQAEC gates,
  Fig.13.5 gate-return counting, and separate interrupt requests. Conflicting
  module-stop descriptions and phase apertures remain explicitly identified.
- §18, Table 18.2/Fig.18.2: comparator ladder/hysteresis; CMDR read-armed baseline
  and interrupt behavior. §1.3/§8.2 identify VCref as P30. The model uses the maximum
  documented response time as its nominal delay.
- ADE-602-053A MOV.B/W/L usage notes (printed pp.121/123/125): update an aliased
  predecrement address register before capturing store data. Instruction encoding
  tables provide fixed displacement-24 selectors and 6B long/CCR restrictions.


## Hardware rationale

These notes preserve source interpretation and model choices that are not obvious from
the code, including the reasoning behind inferred behavior and nominal parameters.

| Mechanism | Reference |
| --- | --- |
| CPU fetch, access order, exceptions and admission | [Execution](research/h8-execution.md) |
| Encoding and arithmetic | [Encodings and arithmetic](research/h8-encoding-and-arithmetic.md) |
| Clocks and SCI/IrDA | [Clock and serial rules](research/h8-clock-and-sci.md) |
| Timers, RTC, watchdog, ADC and AEC | [Counter and converter rules](research/h8-counters-and-adc.md) |
| Register bus and GPIO | [Access and pin rules](research/h8-registers-and-gpio.md) |
| SSU and IIC2 | [SSU](research/h8-ssu.md), [IIC2](research/h8-iic2.md) |
| Internal flash | [Flash](research/h8-flash.md) |
| Supply and reset | [Board supply, retention and startup](research/power-and-reset.md) |
| ADC board circuit and comparators | [Battery sensing](research/adc-board-transfer.md), [comparators](research/h8-comparators.md) |
| BMA150 sensor | [Sampling, filtering and register behavior](research/bma150-behavior.md), [I²C](research/bma150-i2c.md) |
| LCD and EEPROM | [Controller and storage behavior](research/lcd-and-eeprom.md) |
| Infrared board behavior | [Optical interface and firmware evidence](research/infrared.md) |

Native save state field semantics are in [SAVE_STATES](SAVE_STATES.md) and
[CPU_STATE](CPU_STATE.md). The [design](DESIGN.md) owns architectural decisions;
comparisons with other emulators support those decisions without defining Pokéwalker
hardware behavior.
