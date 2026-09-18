# H8/38606F CPU completion audit

Research handoff, 2026-09-18. Reviewed the current starter at
`daf72808f3fb1e7128e82679042fcca3570b1405`, particularly `cpu/decode.rs`,
`cpu/alu.rs`, `cpu/mod.rs`, interrupt integration, and the independent
`hachiware` guest cases. This is a source audit; no emulator run supplied an
expected result, and no production or suite files were changed.

The concrete defects identified by this audit are interrupt admission and exception state,
not a missing instruction family. The earlier current-starter
[execution](h8-execution.md) and [encoding](h8-encoding-and-arithmetic.md)
notes contain fixes that are now present; their implementation-gap descriptions
should not be read as the present status.

## Implementation checkpoint

All four corrections below are implemented. The original findings describe the
reviewed revision, before those corrections. Peripheral enable bits cleared by
an instruction remain eligible only through its admission boundary; live source
flags and CCR masking still decide admission. IRQ mux protection ages at real
instruction boundaries, not host calls. Reset retains a pending NMI while its
first instruction executes. Exception entry duplicates CCR; STC.W remains separate.

`out/cpu-admission-check` passes the complete checks and 100 independent guest
diagnostics. `tests/interrupt_admission.rs` additionally checks masked expiration
and half-microsecond partitions with restoration at every boundary. Full retail
replay and snapshot checks ran in `out/cpu-admission-retail`. The only changed
export byte was idle RAM `FF69`, from `00` to `08`, now matching saved CCR at
`FF68`; the reviewed regression hash was updated for that consequence. All four
saved workload reports and exports were then checked against that baseline.

## Primary sources and applicability

- [H8/38602R hardware manual, REJ09B0152-0300 rev. 3][hardware]: target
  instruction restrictions, normal-mode addresses, and interrupt behavior.
  [H8/38606 addition, TN-H8*-A414A/E][addition] supplies the target changes;
  it does not replace the CPU instruction set.
- [H8/300H software manual, REJ09B0213-0300 rev. 3][software]: encoding
  tables §§2.4–2.5, CCR table §2.7, physical access sequences §2.8, and
  exception processing §3.
- [Renesas Technical Q&A, REJ05B0521-0200][qa]: CPU clarifications
  QA300H-015A, -016A, -021A, -033A, -036A, and -037A. Its peripheral examples
  cover older H8/300H products, so target-specific peripheral rules prevail.
- [H8/3318 hardware manual §2.3.2, p. 25][old-stack]: the H8/300 normal-mode
  stack explicitly inherited by H8/300H. This is manufacturer architectural
  evidence, not another emulator's implementation.

Cached manuals are `out/research/h838602r-hardware.*` and
`out/research/h8300h-software.*`. Their PDF page numbers are printed pages +34
and +16 respectively; Q&A PDF pages are printed pages +18.

## Ranked corrections

### 1. Preserve the first instruction after reset, including against NMI

`Cpu::reset` inherits `interrupt_delay = 0`; its vector/target-fetch sequence
ends at an ordinary `Boundary`. Consequently an already latched NMI can enter
before the reset program initializes ER7. [Target §3.2.2, p. 45][reset]
requires that first instruction to execute. [Q&A -021A, p. 23][reset-qa]
distinguishes sampling an NMI after reset release from admitting it after that
instruction.

Set reset-specific admission deferral for one instruction without suppressing
the pending request. Do not apply the same deferral to every exception target:
that would change ordinary nesting and RTE behavior. This can use the existing
admission state; it needs no alternate executor.

### 2. Keep a request raised during peripheral-enable clearing eligible

`TimerW::write(F0F2)` immediately changes `enable`, and `Mcu::interrupt()`
recomputes `status & enable`. Thus an interrupt raised inside the instruction
that clears its enable disappears before the next CPU boundary.
[Target §3.8.4, p. 60][disable] distinguishes this from clearing the source
flag: disabling takes effect after the instruction's admission opportunity,
whereas a cleared source cancels the request.

Retain the relevant source's eligibility through that instruction, preserve
ordinary priority and CCR masking, and still consult whether its source flag
survives. Register readback should reflect the completed write. Do not retain
an unconditional vector indefinitely or automatically clear a peripheral's
status on admission. A bounded source/enable qualifier is sufficient.

[Q&A -015A, p. 16][disable-qa] corroborates the peripheral case but describes
different cancellation for its older products' external **IER**. That is not
permission to replace the target's IENR contract with the older controller.
Use Timer W for the first diagnostic so this distinction cannot obscure the
CPU/peripheral defect.

### 3. Save the second CCR byte during exception entry

`Phase::ExceptionCcr` currently writes `CCR << 8`, so a saved `35` becomes
`3500`. [Target Fig. 3.5 and §3.8.1, pp. 57–58][stack] label both bytes CCR;
the asterisk means the odd byte is ignored **on return**. The
[software manual §1.1, pp. 3–4][normal-stack] explicitly inherits H8/300's
stack representation, whose [manufacturer description][old-stack] duplicates
CCR to make the word.

The strongest supported implementation is `CCR * 0x0101` for exception
stacking. It follows the target diagram plus an explicit inherited rule;
there is no reason to privilege zero filling. Preserve the existing PC-first,
CCR-second write order and capture CCR before setting I.

Do not turn this into a blanket STC.W requirement: [Q&A -037A, p. 40][stc-qa]
explicitly leaves STC's odd-address byte unspecified. Its current deterministic
zero is a separate model choice. LDC.W/RTE still consume the even byte only.

### 4. Account for IRQ pin-function switching and its clearing delay

This is adjacent MCU integration, not an ALU change. `Control::pins` only
detects transitions between two selected levels. Selecting a low IRQ input
from `None` therefore never sets its flag, and IRR writes have no settling
qualification. [Target §3.8.2, p. 59][mux] documents the mux-generated flag
and requires an intervening instruction before clearing it; holding the
affected inputs high avoids the flag.

Notify the interrupt owner of actual IRQ function changes in PFCR/PMRB and
the corresponding AEC selection. Model the low-input change and one-instruction
clear restriction directly. This is a chosen compact realization of the
documented instruction-level effect, not a claim about an undocumented analog
delay. Do not generate a new flag from every unchanged register write or every
GPIO synchronization.

## Original distinguishing diagnostics

These expected observations follow from the cited rules and ordinary address
arithmetic. They are proposed new diagnostics, not hardware captures. Mask
unrelated sources and keep handlers from changing the observed stack bytes.

| Case | Guest setup and stimulus | Independently expected observation |
| --- | --- | --- |
| Reset admission | Reset vector `0100`; first instruction `7A07 0000 FF70` (`MOV.L #FF70,ER7`); vector 7 points to `0200`. Assert NMI after reset release during the vector/initial-target fetch, before the first instruction begins. | First MOV completes. Then one NMI saves return PC `0106` at `FF6E`; exception SP is `FF6C`. No exception stack write occurs before ER7 initialization. |
| Disable race | I=0, ER0=`F0F2`, Timer W IMIEA=1 and IMFA=0. Execute `7D00 7200` (`BCLR #0,@ER0`). Arrange a GRA match after its operand read but before its write, during the intervening NEXT prefetch. | Enable bit 0 reads zero; IMFA remains one; vector 35 is admitted at instruction completion. The saved return PC is the BCLR address +4. |
| Source-clear control | Repeat with ER0=`F0F3`, IMIEA remaining set, and the match during instruction decoding **before** the BCLR operand read. That read observes IMFA=1 and qualifies its clearing. | IMFA becomes zero; no Timer W entry. This checks source cancellation and respects TSRW's read-before-clear rule. |
| CCR exception word | SP=`FF70`, CCR=`35`, execute `5700` (`TRAPA #0`) at `0100`; vector 8 points to a handler that preserves this memory. | `FF6C..FF6F = 35 35 01 02`; SP=`FF6C`. Observed physical order remains PC word at `FF6E`, then CCR word at `FF6C`. |
| IRQ mux settling | I=1, PFCR=0, PMRB=0, IRQ0 flag clear, PB0 held low. Prepare R1L=`01`, R0L=`FE`; execute `6A89 FFCA` (select IRQ0 in PMRB), immediately followed by `38F6` (clear IRR1 bit 0). | The IRQ0 flag remains one. Insert `0000` (NOP) between those writes and it clears to zero. Repeat with PB0 held high: the mux change should not create the flag. |

Use a later flag clear in the disable-race handler to prevent deliberate
re-entry from obscuring the count. Place generated edges strictly inside the
named phases, not on a coincident boundary; derive their absolute fixture
times from the documented bus sequence. The mux pair deliberately varies
instruction separation rather than assuming a guessed nanosecond delay.

## Reviewed areas that do not warrant another rewrite

- **Instruction coverage and masks:** the current decoder contains the
  target's listed families. MOVFPE/MOVTPE are explicitly excluded by the
  [target instruction list, pp. 16–23][instructions]. H8S/H8SX extensions are
  not missing target instructions. The formerly rejected MOV.L displacement
  store selector is now accepted and has an independent guest case. Long
  register selectors, fixed prefix fields, memory-bit direction, and the
  zero extension byte in 24-bit forms agree with [encoding tables][encoding].
  This inspection found no additional concrete legal encoding rejected.
- **DAA/DAS and wider flags:** current decimal adjustment matches the defined
  correction rows, retains decimal carry, and derives N/Z from the byte result.
  H/V are not guaranteed. Word/long half-carry uses bits 11/27; INC/DEC preserve
  H/C; ADDX/SUBX preserve cleared Z through a zero byte result; SHAL alone has
  sign-change overflow. These agree with [§2.7][flags]. [Q&A -033A/-036A][alu-qa]
  confirms flag-driven decimal behavior and chained SUBX Z. No predecessor
  tracking or generic all-flags helper is justified. Do not duplicate the
  earlier note's suggested arithmetic grids as though a new defect were found.
- **Prefetch and aliases:** ordered NEXT fetches, target-before-call-stack
  writes, complete ER updates for address predecrement/postincrement, normal
  16-bit address reduction, and register aliases already have implementations
  and focused independent cases. Keep those when fixing admission. The
  independent suite currently exercises RAM self-modification, call-target
  overwrite, predecrement aliases, displacement stores, and division
  continuation; local cases additionally cover RTE/LDC admission and EEPMOV.
- **Unassigned opcodes:** no reviewed primary source establishes a universal
  NOP behavior or an illegal-instruction vector for this target. A decode
  diagnostic describes the model boundary; it must not be presented as a
  guest architectural exception. There is no supported new opcode alias to
  add from this audit.

[hardware]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[addition]: https://www.renesas.com/en/document/tcu/addition-h838606-group
[software]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual
[qa]: https://www.renesas.com/us/en/document/apn/technical-qa-h8300h-series-application-note
[reset]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=79
[reset-qa]: https://www.renesas.com/us/en/document/apn/technical-qa-h8300h-series-application-note#page=41
[disable]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=94
[disable-qa]: https://www.renesas.com/us/en/document/apn/technical-qa-h8300h-series-application-note#page=34
[stack]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=91
[normal-stack]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=19
[old-stack]: https://www.renesas.com/en/document/mah/h83318#page=44
[stc-qa]: https://www.renesas.com/us/en/document/apn/technical-qa-h8300h-series-application-note#page=58
[mux]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=93
[instructions]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=50
[encoding]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=221
[flags]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=245
[alu-qa]: https://www.renesas.com/us/en/document/apn/technical-qa-h8300h-series-application-note#page=54
