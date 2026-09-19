# Memory, register accesses and GPIO

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

The target map contains 48 KiB flash and 2 KiB RAM. Normal-mode effective addresses
truncate to 16 bits; word alignment, longword decomposition and register access widths
follow the physical bus. Register masks, reset values, read side effects and protected
writes are implemented by the relevant peripheral. GPIO resolution includes direction,
output latches, open-drain selection, pull-ups and peripheral pin priority.

The fixed board connects the shared serial bus, chip selects, sensor interrupt,
buttons, buzzer, battery sensing and infrared pins. Selecting an alternate function
on a connected pin can affect another device. The BMA150's I²C uses its existing
P91/P92 wiring; the MCU's IIC2 uses P90/P91. They are different bus connections.

## Limits and open questions

Selected behavior remains for holes, prohibited access widths, reserved mux selectors
and PCR readback. In particular, PCR readback preserves direction latches, consistent
with the matching firmware's read/modify/write sequences despite write-only wording.
Conflicting EEPROM and sensor MISO drivers resolve low. The cited drive strengths
support that nominal choice but do not determine contention voltage or damage.
Analog fixtures use Vcc/2 as their digital threshold. Firmware relying on floating
pins, marginal levels or prohibited accesses reaches these model choices.

## Primary evidence

- [H8/38602R hardware manual, REJ09B0152-0300 rev. 3][manual]: §2.3.2 p. 15;
  §§2.5–2.6 pp. 27–33; §8 pp. 119–144; §20.1 pp. 372–375. PDF page numbers
  are printed page numbers plus 34. Cached as `out/research/h838602r-hardware.*`.
- [H8/38606 addition, TN-H8*-A414A/E][addition], pp. 1–5: applicable memory
  map, flash changes, package, and otherwise inherited specifications.
- [TN-H8*-A287A/E][corrections], p. 3: explicitly requires word reads of ADRR;
  this correction is incorporated into the current manual, p. 350.
- [Japanese hardware manual, RJJ09B0161-0400 rev. 4][japanese], §20.1:
  independent inspection of the manufacturer's register listing.
- Public matching [`pw` at `6dc7bc0`][pw-ir]: actual firmware accesses, cited
  below. Firmware comments and the `TARGET_F088` identifier are interpretations,
  not manufacturer definitions.

## Address decoder and physical accesses

The target is normal mode: the CPU generates a 24-bit effective address and ignores its
upper eight bits. A word/longword access with A0 set starts at the preceding even
address, without an address-error exception. Data is big-endian. These are explicit
rules, not generic assumptions about wrapping. ([§2.3.2][alignment],
[§§2.5–2.5.2][addressing])

Decode physical addresses against flash `0000–BFFF`, RAM `F780–FF7F`, and the assigned
registers in the two I/O windows `F020–F0FF` and `FF80–FFFF`. The other ranges, and
unassigned slots inside the I/O windows, are holes. Do not wrap an offset within the 48
KiB flash or 2 KiB RAM. Apply the 16-bit physical-address reduction to each subaccess:
the second word of a longword beginning at `FFFE` is at `0000`; a word beginning at
`FFFF` instead aligns to `FFFE`. ([Addition p. 2][map]; subaccess wrapping is the direct
consequence of the documented address reduction and ordered accesses.)

RAM/flash have a 16-bit bus and allow byte/word transactions in two reference clock
states. Eight-bit peripheral registers allow a word instruction as two successive byte
bus cycles, each with that address's timing and effects. A native word peripheral uses
one two-state transaction. Preserve completed earlier accesses even if reset occurs
before a later one. ([§2.6][bus], [register address/width table][registers])

| Native word register | Address | Documented access |
| --- | --- | --- |
| Timer W TCNT | `F0F6` | Read/write, word only; reset `0000`. |
| Timer W GRA/B/C/D | `F0F8/FA/FC/FE` | Read/write, word only; reset `FFFF`; capture/buffer semantics still apply. |
| AEC ECPWCR | `FF8C` | Read/write, word only; reset `FFFF`. |
| AEC ECPWDR | `FF8E` | Write only, word only; reset `0000`; read value unspecified. |
| ADC ADRR | `FFBC` | Read only, word only; conversion result in bits 15:6. |

Sources: [Timer W §§10.3.7–8, pp. 164–165][timer],
[AEC §§13.3.1–2, pp. 216–217][aec], [ADC §17.3.1, p. 350][adc].
ECH/ECL at `FF96/FF97` remain separate eight-bit registers even in 16-bit counter mode.
Do not turn their word instruction into a native atomic sample. Also retain mixed
timing: a word at `FFA6` reads SEMR then IrCR in 3 + 2 data-access states. ([Register
table][registers])

### Selected behavior for prohibited byte accesses

The target manuals prohibit these byte accesses but specify neither a CPU fault nor
their resulting data. Related H8/300H manufacturer bus diagrams establish the ordinary
lane geometry: even byte addresses select D15:8, odd addresses D7:0; reads share a
strobe while writes have separate lane strobes. They also mark the other write lane's
data as undetermined. This is useful structural evidence, not proof of the target
peripheral's write qualification. ([H8/3048B §6.3.3, table 6.4, p. 136][lanes])

The selected byte-access rule is that a byte read samples the selected high/low lane of
the owner's currently readable word, in two states; a byte write does not qualify the
word-only write latch and has no effect. Do not manufacture a read-modify-write of the
other lane. ADRR writes remain ineffective at either width. ECPWDR returns the existing
chosen zero at either width. Two byte reads are independent samples and gain no new
anti-tearing latch.

For Timer W, obtain the readable word through its owner, including the delayed
visibility of a captured value; slicing a raw debug counter bypasses that behavior. This
choice models a common read path with a full-word write enable. It is intentionally
distinct from claiming byte access is hardware-supported; partial-lane writes or
full-word corruption would require different evidence.

## Holes, reserved fields, and F088

The manufacturer's general precaution on undefined addresses gives no promised read
constant, bus-retention behavior, or write effect. It warns that some addresses may
contain test/future functions. That does not document an emulated fault mechanism.
([Manual, introductory precaution 4][holes])

Selected hole rule: return `00`, discard writes, and finish an ordinary two-state byte
cycle. An unassigned target does not assert the native-word selection, so a word is two
such cycles. This is a stateless default-decoder choice, not a measured pull-down or a
claim that real holes all read zero. Do not add RAM backing, a last-bus-value latch, or
a new CPU exception. Route specified but unfinished peripherals to their owners; holes
must not silently absorb IIC2, flash-control, or any other assigned register.

`F088` is absent from both inspected manufacturer register maps and the target addition.
[`IrInitPins`][pw-ir] writes `03` there, then writes PDR3=`01` and PCR3=`05`. This
establishes an executed access, not a two-bit register, readback, or a pin-control
function. F088 follows the hole rule without a separate latch. Do not infer extra
infrared inversion, pull-ups, or drive strength from the neighboring firmware writes.

Do not apply one universal reserved-bit mask. PFCR bits 7:5 explicitly read zero and
cannot change; most absent GPIO bits have unspecified reads and cannot change. In
contrast, target EBR1 bits 7:6 are explicitly readable/writable despite the instruction
to write zero. Keep their storage, with no invented functional effect. The same
distinction already applies to reserved writable AEC bits. ([PFCR p. 143][pfcr],
[addition p. 4][ebr])

## GPIO register and pad rules

The ordinary implemented masks are:

| Register group | Mask |
| --- | --- |
| PDR/PCR/PUCR 1 and 3 | `07` |
| PDR/PCR/PUCR 8 | `1C` |
| PDR/PCR/PUCR 9, PODR9 | `0F` |
| PDRB | `3F`, read only |
| PMR1 / PMR3 / PMRB / PFCR | `3F / 01 / 0B / 1F` |

All implemented GPIO control/output latches reset to zero. Input reads still come from
the attached board. Discard writes to unimplemented bits; choose zero for their
unspecified read values. ([§§8.1–8.6][gpio])

- PDR reads use the output latch when the corresponding PCR bit is one and
  the resolved pad when it is zero. Alternate-function output levels do not
  replace that documented output-latch readback.
- PCR is documented write only, with unspecified reads. Keep direction-latch
  readback as the chosen rule: retail [`BeepInit`][pw-beep] and
  [`BatterySample`][pw-battery] perform PCR8 read-modify-write operations. This
  makes their behavior coherent without treating firmware usage as proof of
  every readback bit.
- PDRB writes have no pin effect. Retail [`InputInit`][pw-input] actually does
  `PDRB |= 20`, so an input-only write is an ordinary completed access.
- Only the ADC channel selected by AMR is specified to read zero in PDRB. Page 140 explicitly keeps
  PB4/COMP0 and PB5/COMP1 as concurrent functions when AMR is not 8/9; enabling
  CME alone does not select a different PDRB read path. Apply this to guest
  reads and debug inspection. ([§§8.5.1, 8.5.3][portb])
- A pull-up is enabled only by `PUCR=1` and PCR=0. Keep this condition when
  an alternate input overrides output direction; merely selecting that input
  does not assert its pull-up. An open-drain high means release, so the board
  resolves the level. GPIO output readback still follows PDR's latch rule.
  ([§8 pull-up tables; PODR9 §8.4.3][pulls])

The board model gives the P10/P12/P90 chip-select nets pull-ups. Other released digital
nets follow connected drivers and enabled MCU pulls, then default low. These defaults
describe the selected digital circuit.

EEPROM Q and the BMA150's four-wire SDO share P93. If both devices drive opposite
levels, the nominal digital model resolves low and continues both serial transfers.
The electrical basis is their stronger specified sink loads: [Bosch table 9][bma-drive]
specifies 0.4 V at 3 mA sinking and a 0.4-V drop at 1 mA sourcing; [ST tables
16–18][eeprom-drive] specify 0.4 V at 1.5 mA sinking and 0.8 Vcc at 0.4 mA sourcing
at the 2.5-V test point. This supports a small deterministic default for conflicting
outputs. Those limits are not current/voltage curves, so this rule does not establish
the actual contention voltage or model heating and supply droop. Keep the choice local
to the shared external data net. Component drive curves, circuit evidence or loaded
output measurements can refine it. Deselecting a device releases its driver immediately;
neither parser loses progress merely because the other device also drives the net.

Analog fixture voltages project to digital input levels at Vcc/2. This is a selected
threshold for the digital fixture interface, without a pad-loading or input-hysteresis
model.

For off-sequence mux settings, retain written implemented bits. The selected decoder rules are: PFCR IRQ selector `11` connects no IRQ source; PMR1 clock
selector `111` releases the alternate output. Do not throw host errors or substitute a
different legal source. These two decoder outcomes are chosen rules; the manufacturer
labels the encodings prohibited. ([§8.1.4][pmr],
[§8.6.2][pfcr])

## Access examples

The first group checks documented behavior and consequences. State counts below cover
data accesses, excluding instruction fetch/decode overhead.

| Guest setup/action | Independently expected result |
| --- | --- |
| RAM `F780..F783 = 12 34 56 78`; word load from `F781` | `1234`, one two-state word access at `F780`. |
| Byte load from 24-bit EA `12F781` | `34`, identical physical target to `F781`; not flash/RAM-size wrapping. |
| Word read `FFA6` | SEMR sampled first, IrCR later; five data-access states. |
| P91 in GPIO mode; PCR9 bit 1=1, PDR9 bit 1=1, PODR9 bit 1=1; external source pulls P91 low | Pad is low, PDR9 bit 1 reads one. Clear PCR9 bit 1: PDR9 reads zero. |
| PDRB pad PB4 high, AMR=0, comparator 0 enabled | PDRB bit 4 remains one. Set AMR=8: that bit reads zero. |
| Write PDRB=`FF` while its pads are held low | No pad changes and implemented input bits still read zero. |
| Write PFCR=`FF` | Readback `1F`; upper fixed-zero bits do not become storage. |

These next cases specify the selected completion rules, not independent measurements of
undocumented silicon:

| Guest setup/action | Selected expectation |
| --- | --- |
| Longword load beginning `FFFE` | Byte accesses `FFFE`, `FFFF`, then the word at `0000`; six data-access states under the chosen narrow hole decoder. |
| Counter stopped, Timer W GRA=`1234`; byte reads at `F0F8/F0F9` | `12/34`, each two states; byte write `AB` to either lane leaves GRA=`1234`. |
| Completed ADC code `155` hex | ADRR word=`5540`; byte reads `FFBC/FFBD` give `55/40`; writes have no effect. |
| Write/read `F088`, using both `03` and `FC` | Writes finish; each read is `00`; no stored F088 state or electrical change. |
| Word write `AB12` at `F084` | First hole write is discarded; second write sets PFCR=`12`; four data-access states. |
| PCR8=`0C`; read-modify-write OR `10`, then AND `EF` | Direction becomes `1C`, then `0C`, preserving the buzzer pin directions. |

## Implementation and checks

The [MCU](../../crates/hs-core/src/mcu/mod.rs),
[GPIO resolver](../../crates/hs-core/src/mcu/gpio.rs) and
[board composition](../../crates/hs-core/src/machine.rs) own routing and physical reads.
Hachiware's [bus cases](https://github.com/lumirth/hachiware/blob/main/cases/bus.py)
check native-word lanes, holes, mixed accesses and comparator pin readback. Some of
those expectations deliberately describe the selected behavior for prohibited accesses.
The [register-access test](../../crates/hs-core/tests/register_access.rs) checks the
execution boundary, while [clock and pin tests](../../crates/hs-core/tests/clock_obligations.rs)
include pull-up behavior. Neither a resolved Boolean nor a correct register read
establishes analog contention voltage.

[bma-drive]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=27
[eeprom-drive]: https://www.mouser.com/datasheet/2/389/m95512-w-955061.pdf#page=35
[manual]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[addition]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=2
[corrections]: https://www.renesas.com/en/document/tcu/h838602-group-specification-changes#page=4
[japanese]: https://www.renesas.com/ja/document/mah/h838602r-group-hardware-manual#page=354
[alignment]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=49
[addressing]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=61
[map]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=3
[bus]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=66
[registers]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=406
[timer]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=198
[aec]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=250
[adc]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=384
[lanes]: https://www.renesas.com/en/document/mah/h83048b-group-hardware-manual#page=164
[holes]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=6
[pfcr]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=177
[ebr]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=5
[gpio]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=153
[portb]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=172
[pulls]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=168
[pmr]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=156
[pw-ir]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/ir.c#L65-L70
[pw-beep]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_buzzer.c#L49-L63
[pw-battery]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_battery.c#L50-L86
[pw-input]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_player_input.c#L12-L35
