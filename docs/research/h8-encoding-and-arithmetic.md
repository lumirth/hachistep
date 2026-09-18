# H8 encodings and arithmetic

This note records encoding and flag rules where manuals, toolchains or other emulators
disagree. [Execution timing](h8-execution.md) covers bus sequencing and exceptions.

## Sources

- H8/300H software manual, REJ09B0213-0300, Rev.3: §1.4 (registers),
  §1.6.4 (encoding fields), §2.2 (individual instructions), §2.4–2.5 (encodings),
  §2.7/Table 2.9 (flags).
  [Original manual](https://www.renesas.com/en/document/mah/h8300h-series-software-manual).
- H8/38602R hardware manual, REJ09B0152-0300, Rev.3: §2.2.3 (CCR),
  §2.4 (target instruction set), §3.1 (exceptions), Appendix A.1 (flags).
  [Original manual](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual).
  The [H8/38606 addition](https://www.renesas.com/en/document/tcu/addition-h838606-group)
  does not replace these CPU rules.
- GNU binutils, commit `8ea833b706790cbf50ef3b46028a9ea0d8ecd462`:
  [opcode definitions](https://gnu.googlesource.com/binutils-gdb/+/8ea833b706790cbf50ef3b46028a9ea0d8ecd462/include/opcode/h8300.h),
  [assembler input](https://gnu.googlesource.com/binutils-gdb/+/8ea833b706790cbf50ef3b46028a9ea0d8ecd462/gas/testsuite/gas/h8300/movlh.s),
  [expected encodings](https://gnu.googlesource.com/binutils-gdb/+/8ea833b706790cbf50ef3b46028a9ea0d8ecd462/gas/testsuite/gas/h8300/h8300.exp).
  This establishes an independent toolchain's encodings, not measured chip behavior.
- MAME H8, commit `57018adb9d8cd92949081fade9ad0ba3038dbf37`:
  [instruction definitions](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst),
  [ALU helpers](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.cpp).
  Used to cross-check interpretations, not as the expected-result authority.

Page references below are printed pages. PDF page ordinals are printed page +16 for the
software manual and +34 for the hardware manual. The cached manuals are in ignored
`out/research/`.

## MOV.L displacement-store encoding

GNU binutils emits this prefixed longword store:

```text
0100 7890 6BA0 0000 0020    MOV.L ER0,@(32:24,ER1)
```

With ER1=`0000F800` and ER0=`11223344`, it must store `11 22 33 44` at `F820..F823`,
advance by ten bytes, and set N/Z/V from the transferred longword while preserving the
other CCR bits. GNU's `movlh.s:14` and `h8300.exp:1148` supply these exact bytes; the
individual MOV.L instruction description also specifies this selector bit. ([Software
§2.2.35,
p.127](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=143)).

There is a real manual conflict: its consolidated p.210 row prints zero for the same
longword-store selector, while its word-store row prints one contrary to the individual
word-store description on p.125. GNU emits high-bit-zero for byte/word stores, but
high-bit-one for this longword store. Its current opcode table also accepts the zero-bit
longword alternative. MAME's prefixed MOV.L mask accepts both, whereas its byte/word
masks require zero. ([Conflicting consolidated
table](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=226),
[MAME longword patterns, lines379–396](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst#L379-L396)).

The decoder accepts either selector high bit for `0100 78n0 6BAr ...` when `r<8`, using
`n&7` as the address register. The second operation word distinguishes this form from
byte/word transfers. This reconciles the two manual rows with independent toolchain
support; equivalence on this physical target is an inference. Do not generalize the
relaxed bit to byte/word transfers, CCR transfers, or longword loads: the available
evidence does not justify that.

## Other selector restrictions

The target documents 62 instructions and explicitly excludes MOVFPE/MOVTPE. PUSH/POP are
MOV aliases. The target includes EEPMOV.W, all sixteen branch conditions, memory CCR
transfers, signed multiply/divide, and inverted carry-bit operations. Do not import
H8S/H8SX-only instructions from MAME's shared table. ([Hardware §2.4,
pp.16–23](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=50)).

Register fields are not uniformly three bits: byte registers and word registers have
sixteen selectors; longword and address registers have eight. In particular, E0–E7 are
valid word operands, valid byte multiply/divide destinations, and valid word
multiply/divide sources. ([Software §1.4,
pp.8–10](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=24),
[MULXS/MULXU, pp.130–133](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=146)).

Compact selector grid; `valid` concerns documented instruction encodings, not the
physical outcome of executing an unassigned encoding:

| Words in memory | Expected classification / distinction |
| --- | --- |
| `0100 7890 6BA0 0000 0020` | Valid MOV.L store |
| `0100 7810 6BA0 0000 0020` | Retain accepted zero-bit longword-store alternative |
| `7810 6BA8 0000 0020` | Valid MOV.W E0,@(32,ER1) |
| `7810 6AA8 0000 0020` | Valid MOV.B R0L,@(32,ER1) |
| `0100 7810 6B28 0000 0020` | Invalid longword destination selector 8 |
| `0140 7810 6BA0 0000 0020` | Valid STC.W CCR,@(32,ER1); changing `6BA0` to `6BA1` is invalid |
| `7D10 7070` / `7C10 7370` | Valid BSET / BTST #7,@ER1; swapping their prefixes is invalid |
| `7080` / `7780` | Invalid BSET immediate bit 8 / valid BILD #0,R0H |
| `790F 1234` / `7A08 0000 1234` | Valid MOV.W #1234,E7 / invalid MOV.L destination 8 |
| `50F8` / `52F7` / `52F8` | Valid MULXU.B R7L,E0 / valid MULXU.W E7,ER7 / invalid destination 8 |
| `5700` through `5730` | Valid TRAPA #0..3; other selector nibbles or nonzero low nibble are invalid |
| `0140 6890` | Invalid byte-transfer opcode after the word CCR prefix |

These restrictions come from the instruction rows and operation-code maps, not from
emulator output. ([Software §2.4–2.5,
pp.205–216](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=221)).
The leading byte of the four-byte address/displacement extension is documented as zero;
the remaining 24-bit value is distinct from that reserved encoding byte. Normal-mode
address truncation is not a reason to accept every reserved bit. ([Software §1.6.4,
p.26](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=42)).

## Extended arithmetic

Half-carry uses carry/borrow at bits 3, 11, and 27 for byte/word/longword H; ADDX/SUBX
consume old C and preserve old Z when the result is zero. INC/DEC preserve H/C;
ADDS/SUBS preserve the entire CCR. SHAL alone sets V on a sign change among the
shift/rotate instructions. Logical operations and extensions preserve H/C, set N/Z from
the appropriate result, and clear V. ([Hardware CCR,
p.12](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=46),
[Software Table 2.9, pp.229–232](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=245)).

Do not change SUBX to fresh-Z semantics. Its individual prose on p.185 says that, but
software Table 2.9, the target's Appendix A.1 SUBX row and note (3), and MAME all
specify sticky Z. The target manual supports sticky Z. ([Hardware
pp.447,457](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=481),
[software p.232](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=248)).

Independent diagnostic grid. Numbers are hexadecimal; flags show only `H N Z V C` in
their actual bit positions (`20 08 04 02 01`). Repeat with the remaining CCR bits set
and clear to verify preservation.

| Operation | Destination, source | Initial flags | Result | Final flags |
| --- | --- | --- | --- | --- |
| ADDX.B | `7F,00` | `05` | `80` | `2A` |
| ADDX.B | `FF,00` | `05` | `00` | `25` |
| ADDX.B or SUBX.B | `00,00` | `00` / `04` | `00` | `00` / `04` |
| ADDX.B | `80,FF` | `05` | `80` | `29` |
| SUBX.B | `80,00` | `05` | `7F` | `22` |
| SUBX.B | `00,FF` | `05` | `00` | `25` |
| SUBX.B | `80,7F` | `05` | `00` | `26` |
| ADD.W | `0FFF,0001` | `00` | `1000` | `20` |
| ADD.L | `0FFFFFFF,00000001` | `00` | `10000000` | `20` |
| INC.W #2 | `7FFE` | `21` | `8000` | `2B` |
| DEC.W #2 | `8001` | `21` | `7FFF` | `23` |
| NEG.W | `8000` | `00` | `8000` | `0B` |
| SHAL.B / SHLL.B | `80` | `20` | `00` | `27` / `25` |

## Decimal adjustment

DAA and DAS derive their correction from the operand and incoming H/C. Incoming N is the
sign flag, not an operation selector; the opcode selects the operation. Both replace N/Z
from the adjusted byte. DAA retains old C unless the correction sets it; DAS retains C
even when its correction addition carries. I/UI/U remain unchanged. H/V are unspecified;
the model preserves them, and hardware conformance masks them out. ([Software
§§2.2.23–24](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=92)).

The manufacturer's QA300H-033A confirms that the incoming flags determine the operation.
INC/DEC are unsuitable predecessors because they leave H/C stale. No
predecessor-instruction history belongs in decimal adjustment. ([Manufacturer Q&A,
p.36](https://www.renesas.com/us/en/document/apn/technical-qa-h8300h-series-application-note#page=54)).

The software manual's §2.7 summary marks DAS C as changing. Its specific DAS rule and
all four rows preserve C, as does the target hardware manual's appendix. Those agree on
the instruction-specific rule. ([Target appendix,
p.448](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=482)).

The printed DAA table omits twenty reachable states: old C=1, upper nibble=0, and either
H=0 with low nibble 0..F or H=1 with low nibble 0..3. For example, packed-BCD `80+80`
produces byte `00`, H=0, C=1 before DAA, and must adjust to `60`, C=1 to represent
decimal 160. The correction rule includes these states. This follows from the
instruction's stated BCD purpose, also supported by MAME's broader comparison bounds;
literal omission from the table should not break valid decimal addition.

Minimal decimal grid; assert result/N/Z/C and CCR user/I preservation, without treating
an arbitrary H/V value as hardware truth:

| Operation | Byte before | H,C before | Byte after | C after |
| --- | --- | --- | --- | --- |
| DAA | `09` / `0A` / `03` | `0,0` / `0,0` / `1,0` | `09` / `10` / `09` | `0` |
| DAA | `A0` / `9A` / `A3` | `0,0` / `0,0` / `1,0` | `00` / `00` / `09` | `1` |
| DAA | `10` / `1A` / `13` | `0,1` / `0,1` / `1,1` | `70` / `80` / `79` | `1` |
| DAA | `00` | `0,1` | `60` | `1` |
| DAS | `00` / `06` | `0,0` / `1,0` | `00` / `00` | `0` |
| DAS | `70` / `66` | `0,1` / `1,1` | `10` / `00` | `1` |

MAME calls ordinary `do_add8` for the adjustment. That incorrectly clears C for the
valid DAA case `10,H=0,C=1`, and sets C for the valid DAS case `06,H=1,C=0`. Use it as
evidence for possible internal arithmetic, not an oracle for documented decimal flags.
Arbitrary non-BCD inputs outside the printed conditions have no guaranteed result; the
current consistent correction rule is not a demonstrated defect. ([MAME
DAA](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst#L1022-L1045),
[DAS](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst#L1381-L1399),
[ordinary add flags](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.cpp#L707-L727)).

## Unassigned encodings

The target exception list defines reset, explicit TRAPA, direct sleep transition, and
interrupts; it does not define an illegal-opcode or arithmetic-fault vector.
MOVFPE/MOVTPE are unavailable on this LSI even though they appear in the generic
software manual. Neither fact establishes the physical behavior of every unassigned bit
pattern. ([Hardware §3.1,
pp.41–43](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=75),
[target exclusions, p.17](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=51)).

`Decode::Invalid` becomes an emulator `Error::Decode`, with PC and fetched words.
Preserve its meaning as an unmodeled encoding diagnostic; do not describe it as a
chip-generated trap, or replace every unassigned pattern with an invented NOP. MAME's
`illegal()` also stops execution, which supplies no physical evidence. ([MAME illegal
handler](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.cpp#L608-L612)).
