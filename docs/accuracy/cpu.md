# CPU execution and exceptions

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

The H8/300H interpreter supports the target instruction families, register widths,
addressing forms, arithmetic flags, branches, stack operations and exceptions. The
decoder applies the H8/38606 instruction set rather than admitting the extra H8S/H8SX
instructions in shared toolchain tables. RAM and flash execute through the same CPU.
Unassigned encodings produce a host decode diagnostic; the model does not establish
their physical outcome.

Execution retains instruction prefetch, partial operations and physical access order.
This covers writes to prefetched RAM code, source/address aliases in predecrement
stores, normal-mode addressing, discarded fetches and exception stack writes.
Interrupt handling includes reset's first-instruction deferral, enable-write races,
CCR instruction deferral and EEPMOV.W's NMI boundaries. Target manuals, Renesas Q&A,
GNU assembler encodings and independent MAME code support the interpretations.

Some source conflicts have specific resolutions: sticky SUBX Z follows the target
flag table; DAA includes reachable decimal states omitted from the printed table;
MOV.L stores accept the alternative selector supported by the conflicting manual and
toolchain encodings. Their reasoning is recorded in [encodings and arithmetic](cpu-arithmetic.md).

## Limits and open questions

Division by zero retains the destination, overflow retains truncated result fields, and unspecified
decimal H/V flags retain their old values. The documented flags have a firmer basis
than these selected result bits. Firmware depending on unspecified results or
unassigned encodings needs further investigation.

## Sources

The H8/38602R hardware manual supplies target-specific timing and interrupt rules; the
H8/38606 addition changes memory, flash, packaging, and electrical conditions without
replacing the CPU architecture. The H8/300H software manual supplies detailed ordered
bus sequences, encodings, and flags. Printed pages are PDF pages minus 34 for the
hardware manual and minus 16 for the software manual. Apply target-specific rules before
examples from older H8 products.
[Hardware manual][hardware], [target addition][addition], [software manual][software].

## Prefetch and physical accesses

Software-manual §2.8/Table 2.10 places NEXT prefetch before MOV data accesses and
between a modifying memory-bit instruction's read and write. Retain the fetched word and
address: RAM code that overwrites that word must still execute the prefetched value.
Extension words, discarded fetches, operand accesses, and internal waits have distinct
effects and suspension points.

For Bcc16, use displacement-word fetch, two internal states, then fetch the selected
next PC. This interpretation reconciles the ordered table with its I=2/N=2 totals: an
untaken target alone cannot supply the next opcode. For Bcc8, retain NEXT and fetch the
potential target, selecting the retained result by the condition. The untaken target
interpretation is supported by MAME's independent implementation. [Sequences,
pp.235–240][sequences],
[branch counts, p.220][branch-counts], [MAME branch macros][mame-branch].

With instruction/data/stack in two-state on-chip memory, the target totals are:

| Operation | States |
| --- | ---: |
| MOV.B/W postincrement or predecrement, including PUSH.W/POP.W | 6 |
| MOV.L postincrement/predecrement, including PUSH.L/POP.L | 10 |
| JMP @@aa:8 and RTS | 8 |
| RTE | 10 |

An access cannot become an idle delay merely because both last two states. The next
instruction's first fetch is charged in the preceding instruction's sequence; do not
charge it again. [Target cycle tables][counts].

The MOV.L and PUSH.L rows disagree about word order despite identical encodings when the
address register is ER7. Use the common MOV.L sequence: decrement the full register by
four, write the high word at EA, then the low word at EA+2. The generic row and MAME
agree; the reversed PUSH row is treated as a table error. For source/address aliases,
the manual explicitly requires the updated register value. That requirement overrides
MAME's old-source choice.
[Alias notes][aliases], [contradictory rows][push-rows], [MAME MOV.L][mame-mov].

## Reset and exceptions

Reset has its own vector, internal-delay, and initial-target-fetch sequence. Interrupt
entry includes the discarded fetch, internal phases, PC stack write, CCR stack write,
vector read, and handler fetch; the normal response is 14 states after the running
instruction. Sleep recovery replaces a fetch with internal work. TRAPA also uses the
detailed 14-state sequence: its instruction definition, cycle table, and ordered
sequence agree, while the target appendix omits four internal states. [Hardware §§3.2,
3.6–7][hardware],
[TRAPA definition][trapa], [software entry sequences][entry].

Execute the first instruction after reset before admitting even a pending NMI. Retain
the request during that deferral. Do not impose this reset-specific rule on every
exception handler. [Target §3.2.2][reset], [Renesas Q&A -021A][reset-qa].

Exception entry writes PC first, then a word with CCR duplicated in both bytes. The
return path ignores the odd CCR byte, but that does not make its stored value arbitrary:
target Figure 3.5 labels both bytes and the inherited H8/300 normal-mode format
explicitly duplicates it. STC.W is separate; Q&A -037A leaves its odd byte unspecified,
and the model selects zero. LDC.W/RTE consume the even byte. [Target stack][stack],
[inherited normal-mode stack][old-stack],
[STC.W clarification][stc-qa].

## Interrupt qualification

A peripheral request raised during the instruction clearing its enable remains eligible
through that instruction's admission boundary. CCR masking and the live source flag
still apply: clearing the source cancels the request. Retain bounded source eligibility,
not an unconditional vector. Q&A -015A corroborates the peripheral case, but its
older-product external IER example does not replace this target's IENR rule. [Target
§3.8.4][disable], [Q&A -015A][disable-qa].

Selecting an IRQ function on a low input can set its flag. The documented clear sequence
requires an intervening instruction. Model the mux change and that instruction-level
qualification; unchanged GPIO resolution must not create a fresh request. [Target
§3.8.2][mux]. LDC/ANDC/ORC/XORC's following-instruction mask deferral does not apply to
RTE. EEPMOV.W admits NMI between completed byte pairs; EEPMOV.B and maskable admission
wait for completion. Saved return PC is the following instruction, so firmware
explicitly resumes any remaining copy.
[Target §§3.8.5–6][hardware].

## Division and undefined result bits

DIVXU/DIVXS continue through zero divisors and quotient overflow. Z reflects a zero
divisor; unsigned N follows the divisor sign bit, and signed N follows operand sign
difference, including a quotient truncated to zero. Preserve the unaffected CCR flags.
There is no specified divide exception.
[DIVXS][divxs], [DIVXU][divxu].

For a nonzero divisor, compute widely and retain the destination-width quotient and
remainder on overflow; for zero, retain the destination. These deterministic result-bit
choices follow MAME, while the documented flag rules take precedence over its
signed-zero and word-width mistakes. Refine the chosen result bits when better evidence
warrants it without halting ordinary execution.
[MAME signed division][mame-divs], [unsigned division][mame-divu].

No reviewed source defines a universal NOP or illegal-instruction exception for
unassigned encodings. A model decode error is a host diagnostic, not an invented
architectural exception. The target instruction list also excludes MOVFPE/MOVTPE;
H8S/H8SX additions are not missing H8/38606 instructions. [Target instruction
list][instructions].

## Implementation and checks

[CPU implementation](../../crates/hs-core/src/cpu/),
[CPU continuation contract](../CPU_STATE.md), and
[bus accesses](bus-and-gpio.md) describe execution and its retained state.
Hachiware's [CPU](https://github.com/lumirth/hachiware/blob/main/cases/cpu.py) and
[interrupt](https://github.com/lumirth/hachiware/blob/main/cases/interrupts.py)
cases check aliases, stack contents, RAM execution, prefetch and interrupt admission.
Local [CPU regressions](../../crates/hs-core/tests/cpu_regressions.rs) and
[admission tests](../../crates/hs-core/tests/interrupt_admission.rs) exercise particular
ordering rules. The [arithmetic topic](cpu-arithmetic.md) records flag and encoding
conflicts. A decoder accepting an encoding does not itself verify its bus sequence.

[hardware]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[addition]: https://www.renesas.com/en/document/tcu/addition-h838606-group
[software]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual
[sequences]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=251
[branch-counts]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=236
[mame-branch]: https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst#L27-L40
[counts]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=501
[aliases]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=143
[push-rows]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=257
[mame-mov]: https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst#L349-L361
[trapa]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=202
[entry]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=259
[reset]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=79
[reset-qa]: https://www.renesas.com/us/en/document/apn/technical-qa-h8300h-series-application-note#page=41
[stack]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=91
[old-stack]: https://www.renesas.com/en/document/mah/h83318#page=44
[stc-qa]: https://www.renesas.com/us/en/document/apn/technical-qa-h8300h-series-application-note#page=58
[disable]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=94
[disable-qa]: https://www.renesas.com/us/en/document/apn/technical-qa-h8300h-series-application-note#page=34
[mux]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=93
[divxs]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=99
[divxu]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=107
[mame-divs]: https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst#L779-L859
[mame-divu]: https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst#L1473-L1511
[instructions]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=50
