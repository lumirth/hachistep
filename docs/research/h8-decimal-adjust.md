# H8/300H decimal adjustment

Source audit, 2026-09-18. Read the current design §§7.3/7.5 and CPU completion
audit first. No production changes, builds, or emulator tests were performed.

The current decimal executor has **no identified discrepancy with the documented
rows**. It lives in `crates/hs-core/src/cpu/mod.rs:1076–1098`; `cpu/alu.rs:12`
supplies only its byte N/Z calculation. Keep this instruction-specific flag
handling: ordinary binary ADD/SUB flag updates are not equivalent.

## Primary contract

The H8/300H software manual REJ09B0213-0300, §2.2.23, printed pp. 76–77
([PDF pp. 92–93][daa]), and §2.2.24, printed pp. 78–79
([PDF pp. 94–95][das]), give the following rules:

- Correction depends on the operand and incoming H/C. Incoming N is the sign
  flag, not an add/subtract selector. The opcode selects DAA or DAS.
- Both instructions replace N with result bit 7 and Z with the zero-result
  predicate. Incoming N/Z do not participate in correction; Z is not sticky.
- DAA sets C on a correction carry out of bit 7, otherwise retaining old C.
  DAS retains old C, including when its unsigned correction addition carries.
- I/UI/U remain unchanged. H/V have no guaranteed result. The current core
  retains H/V as its deterministic realization; conformance must mask them out.

The manufacturer Q&A QA300H-033A, printed p. 36
([PDF p. 54][qa]), confirms that execution follows flag state. INC/DEC are
unsuitable predecessors because they leave H/C stale, not because decimal
adjustment recognizes the preceding opcode. No predecessor history is needed.

The software manual's §2.7 summary, printed p. 230, marks DAS C as changing.
Use the specific §2.2.24 rule and all four rows, which preserve C. The target
H8/38602R hardware manual's appendix, printed p. 448
([PDF p. 482][target-das]), independently marks DAS C unchanged.

## Complete documented rows

Rows are numbered here in their printed order. Every value/range is hexadecimal
and inclusive. `Upper` and `Lower` are the incoming byte's nibbles. Compute the
expected byte as `(input + Add) modulo 256`, using the literal row's correction;
take expected C directly from its final column. These tables are independent of
the production correction predicates.

| DAA row, p. 76 | C in | Upper | H in | Lower | Add | C out |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 0 | 0–9 | 0 | 0–9 | 00 | 0 |
| 2 | 0 | 0–8 | 0 | A–F | 06 | 0 |
| 3 | 0 | 0–9 | 1 | 0–3 | 06 | 0 |
| 4 | 0 | A–F | 0 | 0–9 | 60 | 1 |
| 5 | 0 | 9–F | 0 | A–F | 66 | 1 |
| 6 | 0 | A–F | 1 | 0–3 | 66 | 1 |
| 7 | 1 | 1–2 | 0 | 0–9 | 60 | 1 |
| 8 | 1 | 1–2 | 0 | A–F | 66 | 1 |
| 9 | 1 | 1–3 | 1 | 0–3 | 66 | 1 |

| DAS row, p. 78 | C in | Upper | H in | Lower | Add | C out |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 0 | 0–9 | 0 | 0–9 | 00 | 0 |
| 2 | 0 | 0–8 | 1 | 6–F | FA | 0 |
| 3 | 1 | 7–F | 0 | 0–9 | A0 | 1 |
| 4 | 1 | 6–F | 1 | 6–F | 9A | 1 |

These enumerate 364 DAA and 380 DAS operand/H/C states. The current predicates
reduce to each literal correction over its entire range, and their carry rules
produce the listed C values. Incoming N/Z/V can be independently varied without
changing the expected byte or defined output flags.

## Literal diagnostic vectors

CCR bit order is `I UI H U N Z V C`. `Expected masked` is the exact required
value of `CCR_out & DD`; DD excludes undefined H/V. `Retained H/V` is the full
CCR under the core's present policy, **not a hardware guarantee**. I/UI/U are
set as preservation canaries. These results are derived from the rows above,
not by executing the emulator.

| Operation/row | Byte in | CCR in | Byte out | Expected masked | Retained H/V |
| --- | --- | --- | --- | --- | --- |
| DAA 1 | 09 | DE | 09 | D0 | D2 |
| DAA 2 | 0A | DE | 10 | D0 | D2 |
| DAA 3 | 03 | FE | 09 | D0 | F2 |
| DAA 4 | A0 | D2 | 00 | D5 | D7 |
| DAA 5 | 9A | DA | 00 | D5 | D7 |
| DAA 6 | A3 | FE | 09 | D1 | F3 |
| DAA 7 | 10 | DF | 70 | D1 | D3 |
| DAA 8 | 1A | D7 | 80 | D9 | DB |
| DAA 9 | 13 | FF | 79 | D1 | F3 |
| DAS 1 | 00 | DA | 00 | D4 | D6 |
| DAS 2 | 06 | F2 | 00 | D4 | F6 |
| DAS 3 | 70 | DF | 10 | D1 | D3 |
| DAS 4 | 66 | FB | 00 | D5 | F7 |
| DAA 1, N initially clear | 90 | D2 | 90 | D8 | DA |
| DAA 1, N initially set | 90 | DA | 90 | D8 | DA |

DAA row 7 catches accidentally clearing old decimal carry when the adjustment
does not itself overflow. DAS row 2 catches accidentally using the unsigned
carry of `06 + FA`. The final pair distinguishes N as an output from N as an
operation selector. DAS rows 1/2 also require setting Z from an incoming zero
flag; a sweep with incoming N/Z/V all set should retain these extra cases.
For preservation checks, repeat with I/UI/U cleared as well.

## The carry-range omission and unspecified combinations

The DAA table excludes 20 states reachable by ordinary valid packed-BCD
addition: C=1, upper=0, with either H=0/lower=0–F or H=1/lower=0–3.
For example, arithmetic on decimal operands independently requires:

| Packed-BCD addition | Binary byte / H / C | Required decimal-adjust byte / C |
| --- | --- | --- |
| 90 + 70 = decimal 160 | 00 / 0 / 1 | 60 / 1 |
| 85 + 85 = decimal 170 | 0A / 0 / 1 | 70 / 1 |
| 78 + 88 = decimal 166 | 00 / 1 / 1 | 66 / 1 |

Extending the lower bound of the upper-nibble ranges in DAA rows 7–9 from 1
to 0 covers these states. The core already does so. This is strong inference
from the instruction's stated packed-BCD purpose and ordinary addition, not a
claim that the missing entries are printed or covered by a located erratum.
Keep these cases in actual arithmetic-sequence diagnostics; do not trap them.

For genuinely non-BCD combinations outside the listed rows, pp. 77/79 expressly
withhold guarantees for the adjusted byte and arithmetic flags. The existing
simple comparator/H/C-driven correction is a reasonable deterministic circuit
inference. There is no primary evidence here for replacing it, treating those
inputs as unimplemented instructions, or conditioning the correction on old N.
Undefined H/V preservation remains a selected realization, not a measured fact.

## Implemented diagnostics

`hachiware/decimal_adjust.py` transcribes these rows independently, sweeping
all 744 listed operand/H/C states with incoming N/Z both clear and set. The
arithmetic guest additionally covers the twenty omitted states and thirteen
literal ADD/ADDX/SUB/SUBX/NEG sequences. Guest stores capture CCR immediately,
mask H/V, and then publish the byte and defined flags. This required no core
instruction change. All three guests and the complete 128-case suite pass in
`out/decimal-conformance.json`; the suite's builder/runner contract tests pass.

[daa]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=92
[das]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=94
[qa]: https://www.renesas.com/us/en/document/apn/technical-qa-h8300h-series-application-note#page=54
[target-das]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=482
