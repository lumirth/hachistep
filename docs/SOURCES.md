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

The archive was inspected for a 64 KiB EEPROM and did not contain one. The EEPROM
used in the runs was acquired separately from the user's File Library:

```
EEPROM bytes: 65536
EEPROM SHA-256: 9b9d7ac29b3d27de8fed1aca392c91ec559a53c2c22f2a9c860c980895539008
```

Neither image was patched. Verification checks their hashes before and after
execution. The archive's unrelated source/toolchain material was not copied into
this repository. Private filesystem paths from original metadata are not part of
the maintained documentation. The private delivery contains just the two images,
not the original archive. Image ownership and redistribution rights are unchanged.

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

The archive also carries an SSD1854 datasheet. Its presence is not evidence that
the LCD owner should implement that controller. This starter implements NT7508
commands and the reached `pw` driver behavior. The original PDF bytes are not
redistributed; local document hashes are in `evidence/input-provenance.json`.

## Decompilation evidence

`lumirth/pw` was read at commit
`6dc7bc09950078fa3fe0dffa4dae34e9549a99da`:
https://github.com/lumirth/pw/tree/6dc7bc09950078fa3fe0dffa4dae34e9549a99da

Relevant files include `src/application/pw_accel_bma150.c`, `pw_nt7508.c`,
`pw_eeprom_m95512.c`, `pw_battery.c`, `pw_player_input.c`, `pw_power.c`,
`pw_rtc.c`, `src/support/lib_common.c`, serial/IR setup, and startup/register
headers. These establish reached accesses and software intent. They are not
substituted as host-native routines, and their source comments are not treated
as independent measurements of undocumented hardware.

Examples of concrete integration evidence are active-high button sampling,
separate LCD command/parameter selected intervals, the BMA protected-window
initialization, clock/module setup around SLEEP, battery sampling/polling,
watchdog service and RTC stable reads. STATUS separates established mechanisms
from provisional physical behavior.

## Original implementation and test code

The Rust core, CLI, small Python tools and original seven synthetic diagnostic programs
were written for the starter. Revision 0.2 extends that Git history, adds eight
guest fixtures and independently stated target regressions. Existing HachiStep implementation files and other
emulator source were not copied into this repository. The core-independent test
builder does not derive expected values by calling the production decoder. It is
not a full independently hardware-validated CPU suite.

## Compiler artifact

The sandbox lacked a Rust compiler and direct binary downloads were unavailable.
An existing public `risc0/rust` CI artifact supplied a Linux host toolchain:

```
Commit: e638c6cfea1eff5fbbb24a27e60538e3760d21b8
Workflow run: 29519685716
Artifact: 8385562883
Artifact SHA-256: 3308427d26e29070f007d27bf837098e69920558c19d1334bcec459237469d8c
```

No workflow was launched to acquire it. No user repository or computer was
modified. The toolchain is not bundled, the project requires no fork-specific
feature, and the declared Rust minimum was not separately tested. Build reports
record the actual compiler rather than claiming an upstream stable version.

## Revision 0.2 source anchors

The implementation uses the same primary references; no new physical captures
were obtained. Printed page numbers differ from PDF indices. The named sections
and diagrams are recorded in owner comments and tests.

- REJ09B0152-0300 §20.1, printed pp.372–375: register physical access widths and
  state counts. `hachiware/spec/register_access.tsv` is a separate 95-row
  transcription, not generated from production routing.
- §3.8.5: LDC/ANDC/ORC/XORC following-instruction interrupt deferral; RTE is not
  in that list. §3.8.6: EEPMOV.B versus EEPMOV.W NMI acceptance and saved next PC.
- §§3.4.1/3.5.1 and IEGR: NMI priority/latch and edge selection; §6.3 Table 6.1
  distinguishes low-at-reset bootstrap/debug straps from ordinary user mode.
- §10, particularly input/output timing and §10.7 conflict notes: Timer W
  compare/capture/buffer semantics and external clock. The implementation uses
  reference-edge pipelines; mux glitches and subcycle behavior are not claimed.
- §13: AEC/PWM, CUE/CRC, independent/cascaded counters, clock and IRQAEC gates,
  Fig.13.5 gate-return counting, and separate interrupt requests. Conflicting
  module-stop descriptions and phase apertures remain explicitly identified.
- §18, Table 18.2/Fig.18.2: comparator ladder/hysteresis; CMDR read-armed baseline
  and interrupt behavior. §1.3/§8.2 identify VCref as P30, not P32. A quoted maximum
  response time is used only as a nominal witness, not an exact measured delay.
- ADE-602-053A MOV.B/W/L usage notes (printed pp.121/123/125): update an aliased
  predecrement address register before capturing store data. Instruction encoding
  tables provide fixed displacement-24 selectors and 6B long/CCR restrictions.

The core-independent fixture corpus is maintained in
[ hachiware ](https://github.com/lumirth/hachiware), with no dependency on
`hs-core`. Its initial expectations are documented or reasoned, not physical
captures. The HachiStep adapter exports observations without owning expectations.
