# H8 execution: prefetch, timing, and division

Reviewed the current starter at `21412e8003fe353bdc4a6b1f1edecdc5394cfbc1`.
This is a source review, not a hardware measurement or an implemented fix.

## Sources and applicability

- **H8/38602R hardware manual**, REJ09B0152-0300, Rev. 3.00: §2.6
  (printed pp.32–33), §3.2.1 (44–45), §3.6.1/Fig.3.4 (56), §3.7.1/Table 3.4
  (57), and Appendix A.3/Tables A.3–A.4 (461–471).
  [Original manual](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual).
- **H8/38606 addition**, TN-H8*-A414A/E, Rev. 1.00, 22 April 2009,
  pp.1–5: the listed differences concern memory, flash organization, packaging,
  and electrical characteristics; they do not replace the CPU execution rules.
  [Original update](https://www.renesas.com/en/document/tcu/addition-h838606-group#page=2).
- **H8/300H software manual**, REJ09B0213-0300, Rev. 3.00: §2.2.26
  (DIVXS, printed pp.83–90), §2.2.27 (DIVXU, 91–95), §2.2.62 (TRAPA, 186),
  §2.6 (cycle counts, 217–227), and **§2.8/Table 2.10** (ordered bus cycles,
  233–244). The last section already supplies much of the sequencing sometimes
  mistaken for undocumented behavior.
  [Original manual](https://www.renesas.com/en/document/mah/h8300h-series-software-manual).

Printed page numbers differ from PDF positions: add 34 for the hardware manual
and 16 for the software manual. Copies and extracted text are in ignored
`out/research/`. Searches of Renesas technical updates by device, manual number,
division, and corrections found no CPU-specific update superseding these rules;
that is a search result, not a claim that every historical bulletin was located.

## 1. Replace instruction-at-a-time fetching with the documented prefetch order

The starter clears `words` at every boundary, then reads the current instruction
at `pc`. After decoding, it commits the instruction's effects before beginning
the next fetch. Consequently, it has no retained next-instruction word.
([CPU boundary/fetch](../../crates/hs-core/src/cpu/mod.rs#L283-L311),
[fetch completion](../../crates/hs-core/src/cpu/mod.rs#L452-L479)).

The manual's sequences instead put NEXT prefetch before MOV data accesses and
between the read and write of modifying memory-bit instructions. Eight-bit
conditional branches list both NEXT and EA fetches.
([§2.8, pp.235–240](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=251)).

**Concrete failure:** RAM code overwrites the first word of its immediately
following instruction. The starter fetches the replacement. A word already
prefetched by the real sequence must remain the executed word. The same problem
affects saved state, interrupt return addresses, and the timing of register
accesses even when the final register values happen to agree.

**Correction:** retain fetched material and its address as causal CPU state;
make NEXT fetch, extension fetch, discarded fetch, data access, and internal wait
explicit stages of the existing executor. Consume the retained first word at
the next instruction boundary. Do not simply add another fetch to every current
instruction: the manual charges the next instruction's first fetch to the
current instruction, and double charging would create another timing defect.
Preserve arbitrary-horizon suspension through these stages.

**Conditional-branch interpretation:** for Bcc16, use displacement-word fetch,
two internal states, then fetch the selected next PC: target if taken, fallthrough
otherwise. The selected-PC interpretation is an inference: reading only an
untaken target could not supply the next opcode, and an extra fallthrough fetch
would contradict both the ordered table and its I=2, N=2 cycle count. MAME instead
reads extension, NEXT, and target. That coincidentally totals six states on
two-state memory, but substitutes a read for documented internal work. For
Bcc8, retain NEXT and perform the potential-target fetch, choosing the retained
word according to the condition. That untaken-target address is supported by
MAME, rather than explicitly explained by the manual's EA legend.
([§2.8, p.236](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=252),
[§2.6, p.220](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=236),
[MAME branch macros](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst#L27-L40)).

## 2. Correct missing states and distinguish bus work from delay

The following totals assume code/data/stack in two-state on-chip memory. Starter
totals below are derived from its action sequences and
[`Mcu::access_states`](../../crates/hs-core/src/mcu/mod.rs#L269-L279), not measured.

| Instruction | Starter states | Required states | Missing work |
| --- | ---: | ---: | --- |
| MOV.B/W postincrement or predecrement, including PUSH.W/POP.W | 4 | 6 | Two internal states |
| MOV.L postincrement/predecrement, including PUSH.L/POP.L | 8 | 10 | Two internal states |
| JMP @@aa:8 | 6 | 8 | An instruction fetch |
| RTS | 6 | 8 | An instruction fetch |
| RTE | 8 | 10 | An instruction fetch |

The target's cycle counts establish these deficits.
([Appendix A.3, pp.467–470](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=501)).
The current causes are direct transition into `Memory`, and the shared
`BranchWait` returning `Idle(2)`.
([memory setup](../../crates/hs-core/src/cpu/mod.rs#L730-L759),
[control-transfer actions](../../crates/hs-core/src/cpu/mod.rs#L353-L401)).

**Correction:** assign each instruction family its actual ordered sequence.
An instruction fetch cannot be substituted with two idle states just because
both consume the same duration. Preserve per-access bus timing; do not patch
these deficits with one final instruction-wide delay. Multiplication and
ordinary nonzero division already have the documented nominal totals; their
prefetch placement still needs the first correction.

**Reconcile PUSH.L's contradictory row:** p.241 orders a predecrement MOV.L's
writes at EA then EA+2, but p.242 lists PUSH.L's low word first. These instructions
have identical encodings when the address register is ER7. Use the common MOV.L
sequence: decrement by four, write the high word at the resulting EA, then the
low word at EA+2. This follows the generic MOV row and MAME's implementation;
treating the reversed PUSH row as a table error is a reasoned choice, not a
hardware measurement. Do not invent a distinct stack-register implementation.
For source/address aliasing, the manual explicitly requires storing the
**decremented** value, including PUSH.L ER7; MAME instead captures the old source
and is unsuitable as the expected result for that case.
([MOV encoding and alias notes, p.127](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=143),
[PUSH notes, p.148](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=164),
[contradictory sequences, pp.241–242](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=257),
[MAME predecrement MOV.L](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst#L349-L361)).

## 3. Complete reset, interrupt, and trap sequencing

Current exception entry immediately decrements SP and writes PC, then CCR, then
reads the vector. PC-before-CCR is correct, but the preceding discarded fetch,
internal phases, and handler prefetch are not represented as one complete entry
sequence. Reset paths likewise construct `Cpu` from an untimed vector value.
([entry](../../crates/hs-core/src/cpu/mod.rs#L273-L279),
[entry continuation](../../crates/hs-core/src/cpu/mod.rs#L544-L552),
[reset integration](../../crates/hs-core/src/machine.rs#L374)).

The target diagram establishes two-state fetch/internal phases around the stack
and vector operations; its interrupt-response table accounts for 14 states
after the running instruction completes. Reset has its own vector/internal/
initial-fetch sequence. Sleep recovery replaces the extra prefetch with internal
work. These distinctions must survive suspension and reset.
([Hardware §3.2.1/Fig.3.1](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=79),
[§3.6.1–3.7.1](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=90),
[software §2.8, pp.243–244](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=259)).

**Reconcile a real documentation conflict:** hardware Table A.4's TRAPA row
omits internal states, yielding 10; software §2.2.62 says 14, §2.6 includes four
internal states, and §2.8 places both two-state internal phases. Use the detailed
14-state normal-mode sequence. This is a supported reconciliation, not a reason
to leave traps incomplete.
([TRAPA description](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=202),
[software count](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=242),
[conflicting target row](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=505)).

## 4. Division must not terminate the machine

`muldiv` returns `Unsupported` before completing zero-divisor or overflowing
quotient cases. This invents a terminal emulator outcome for an instruction that
the CPU executes.
([Current checks](../../crates/hs-core/src/cpu/mod.rs#L891-L929)).

DIVXU/DIVXS set Z from a zero divisor; preserve H/V/C and the other unaffected
CCR bits. Unsigned N follows the divisor's sign bit; signed N follows operand
sign difference, including a zero quotient. Zero and overflow destination
results are not guaranteed, and no division exception is specified.
([DIVXS pp.83–87](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=99),
[DIVXU pp.91–93](https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=107)).

**Correction now:** execute the normal timed instruction and commit documented
flags. For a nonzero divisor, calculate in a sufficiently wide type and retain
the destination-width quotient/remainder fields on overflow. For zero, preserve
the destination. These result-bit choices follow an existing independent
implementation and provide a deterministic continuation; they are not claimed
as measured Pokéwalker results. A focused hardware case can refine them without
blocking normal execution or introducing a public accuracy setting.

MAME uses these overflow/zero-result choices, but is not an oracle: its signed
N computation uses `q < 0`, which misses the manual's negative zero-quotient
case, and its word DIVXU tests bit 7 for N. Keep the starter's correct operand-sign
logic and word-width sign mask.
([Pinned MAME signed division](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst#L779-L859),
[unsigned division](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst#L1473-L1511)).

## Implementation order and distinguishing cases

1. Fix division's terminal errors with a small focused change. Cover all four
   signed/unsigned widths, zero divisors, overflowing quotients, operand aliases,
   preserved CCR bits, and negative signed quotients truncated to zero.
2. Add retained prefetch state and ordered continuations, then close the timing
   deficits together. Use RAM self-modification of the next first word, a
   separately changed extension word, and both taken and untaken branches.
3. Complete reset/exception/trap stages. Place reset immediately before and after
   each stack access; compare completed writes, saved PC/CCR, and elapsed states.
4. Compare uninterrupted execution with stops and save/restores around these
   effects. Expected bus order and timing come from the cited tables, not the
   candidate executor. Keep inferred exceptional division result bits separate
   from independently established hardware expectations.

Retail execution remains a useful integration workload, but cannot establish
these distinctions by itself. No alternate executor, rollback, broad observer
framework, or per-instruction allocation is needed for the corrections.
