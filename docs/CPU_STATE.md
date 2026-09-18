# CPU save state semantics

The native representation describes unfinished hardware work and already completed
effects. This document explains those meanings; field declarations and explicit tags
live in [cpu/state.rs](../crates/hs-core/src/cpu/state.rs). The [file
contract](SAVE_STATES.md) defines encoding, validation, and restoration. Changing
executor layout does not change these meanings automatically.

## Common retained fields

Use one private `CpuState` with these fields, followed by a tagged `CpuProgress`.
Integers have explicit widths; addresses are normal-mode 16-bit addresses, not host
pointers. Give saved enums documented tags independently of the live Rust enum order.

| Field | Type and meaning | Live mapping |
| --- | --- | --- |
| `er` | `[u32; 8]`, including the upper halves of address registers and SP | `registers.er` |
| `pc` | `u16`, current architectural next-instruction address | `registers.pc` |
| `ccr` | `u8`, currently committed flags/control bits | `registers.ccr` |
| `instruction_address` | `u16`, address of the instruction whose fetched material is retained | `instruction_pc` |
| `fetched_count`, `fetched_words` | `u8`, `[u16; 5]`; words actually returned by earlier fetches, in order | `word_count`, `words` |
| `prefetch` | `Option<{ address: u16, word: u16 }>`, already fetched and retained next word | `prefetch` |
| `interrupt_deferral` | `u8`, currently 0 or 1 instruction-boundary deferrals | `interrupt_delay` |
| `exception_notification` | `Option<u8>`, admitted vector not yet consumed by MCU integration | `accepted_vector` |

Keep `instruction_address`: `write_origin` uses bit 1 for absolute-8 MOV.B access
provenance. It is not merely a trace label. Preserve prefetch and fetched words even if
the backing RAM/flash has changed. Preserve the full 32-bit ER values: address
arithmetic and updates operate on them before the bus truncates addresses. ([CPU fields
and provenance][cpu].)

`Instruction` is disposable decoded interpretation. For progress that needs it, run the
existing pure `decode` on the captured word prefix and require `Ready` with exactly that
count. Do not read guest memory or call `next`, `prepare`, `begin` or `complete` to
reconstruct it. Retain the small fetched record even where it is only historical; this
avoids fragile phase-specific omission rules and preserves diagnostic attribution.

## Complete progress mapping

The following are concrete suggested saved variants. Each row maps in both directions;
fields named below have their live field's value. A `frame` means `{ vector: u8,
saved_pc: u16, saved_ccr: u8 }`. Where a row mentions `decoded`, reconstruct it from the
captured words. Set inactive live `continuation` to `Boundary`. No serialized variant
contains another arbitrary `Phase`.

| Saved variant and payload | Live phase / active continuation | Meaning at capture |
| --- | --- | --- |
| `ReadResetVector` | `ResetVector` | Vector word at 0 has not completed. |
| `InstructionBoundary` | `Boundary` | Prior instruction has retired; admission/next fetch have not occurred. |
| `FetchInstructionWord` | `Fetch` | Next word is read at `pc & !1`; the saved prefix has already completed. |
| `PrepareFetchedInstruction` | `Ready(decoded)` | Decoding is done; preparation effects are not. |
| `ExecuteFetchedInstruction` | `Execute(decoded)` | Preparation is done; execution setup has not occurred. |
| `FetchNextBeforeExecution { address:u16, retain:bool }` | `Prefetch { address, retain }`, continuation `Execute(decoded)` | Next-word fetch precedes instruction execution. |
| `FetchDiscardBeforeException { address:u16, frame }` | `Prefetch { retain:false, address }`, continuation `ExceptionPc(frame)` | Exception has been admitted; discard fetch remains, followed by two internal states. |
| `FetchNextBeforeBitWrite { address:u16, write_address:u16, result:u8 }` | `Prefetch { retain:true, address }`, continuation `BitWrite` | Bit operand was read and flags/result calculated; writeback remains. |
| `FetchNextBeforeRetirement { address:u16 }` | `Prefetch { retain:true, address }`, continuation `Finish` | Read-only memory-bit operation or EEPMOV completed; final NEXT fetch remains. |
| `CalculateBranchAddress` | `Delay(2)`, continuation `Execute(decoded)` | Two-word Bcc/BSR address-calculation interval before evaluating/starting the branch. |
| `DelayAfterAddressUpdate { transfer }` | `Delay(2)`, continuation `Memory(transfer)` | Address-update instruction has latched its transfer; any pre-decrement is already committed. |
| `DelayBeforeExceptionStack { states:u32, frame }` | `Delay(states)`, continuation `ExceptionPc(frame)` | Exception entry interval: 2 states after discarded NEXT/TRAPA, or 4 from SLEEP. |
| `RetireInstruction` | `Finish` | Instruction effects complete; retirement/boundary transition remains. |
| `TransferMemory { transfer }` | `Memory(transfer)` | One logical memory operand, possibly between its two word transfers. |
| `ReadBitOperand { address:u16, operation:BitOp, bit:u8 }` | `BitRead` | Operand read and flag/result calculation remain. |
| `WriteBitResult { address:u16, result:u8 }` | `BitWrite` | Captured byte is ready for writeback; do not apply the bit operation again. |
| `ReadIndirectTarget { address:u16, call:bool }` | `IndirectJump` | Target pointer read remains. |
| `WriteReturnBeforeTargetFetch { address:u16, target:u16, return_pc:u16 }` | `Call { fetch_target:true, ... }` | Indirect JSR already decremented SP; return-PC write precedes target fetch. |
| `WriteReturnAfterTargetFetch { address:u16, return_pc:u16 }` | `Call { fetch_target:false, target:0, ... }` | Target/PC/prefetch installed and SP decremented; return-PC write remains. Live `target` is now dead. |
| `FetchConditionalTarget { target:u16, install:bool }` | `BranchTarget { target, take:install }` | Branch outcome/address are latched; fetch still occurs even when its result will be discarded. |
| `FetchJumpTarget { target:u16, stack_return:Option<u16> }` | `JumpTarget { target, call:stack_return.is_some(), return_pc:stack_return.unwrap_or(0) }` | Target fetch remains; `Some` requires a later return-address push. |
| `DelayBeforeJumpTarget { target:u16, call:bool }` | `BranchWait` | Two internal states before target fetch; return PC will then be taken from architectural PC. |
| `DelayBeforeEntryTarget { target:u16 }` | `EntryWait` | Reset/exception vector read completed; two internal states remain. |
| `FetchEntryTarget { target:u16 }` | `EntryTarget` | Entry target word has not completed; PC/prefetch installation remains. |
| `ReadReturnPc` | `ReturnPc` | RTS stack word at current SP remains unread. |
| `ReadReturnCcr` | `ReturnCcr` | RTE's first stack word remains unread. |
| `ReadExceptionReturnPc { pending_ccr:u8 }` | `ReturnExceptionPc { ccr }` | First RTE word read and SP increment already committed; CCR is latched but not restored until PC read completes. |
| `WriteExceptionPc { frame }` | `ExceptionPc` | Admission set I and decremented SP; saved PC write remains. |
| `WriteExceptionCcr { vector:u8, saved_ccr:u8 }` | `ExceptionCcr` | Saved PC write and second SP decrement completed; duplicated-CCR word write remains. |
| `ReadExceptionVector { vector:u8 }` | `ExceptionVector` | Both stack writes completed; vector word remains unread. |
| `ArithmeticResultPending { result_size:Size, destination:u8, result:u32, pending_ccr:u8, states:u32 }` | `MulDiv { size, dst, value, flags, states }` | Multiply/divide result is calculated but uncommitted until its 12/20-state interval completes. |
| `CopyBytes { count_width:CountWidth, step:CopyStep }` | `Copy { word_count, stage, value }` | EEPMOV mapping below. |
| `Sleeping` | `Sleeping` | SLEEP has retired; wake/direct-transition work is owned by Machine/control. |

`BitOp` names Set, Clear, Toggle, Test, StoreCarry, LoadCarry, AndCarry, OrCarry and
XorCarry; carry operations retain the existing inversion boolean. These map directly to
`decode::Bit`, not instruction opcode ordinals.

`install` deliberately does not mean "the condition was true": untaken Bcc16 selects
fallthrough and sets live `take=true` to install that fetched word. Raw
branch/entry/indirect targets can be odd; the existing executor aligns the actual fetch
and later architectural PC. Do not reject odd targets or normalize their latched values
prematurely.

`Ready`, `Execute` and `Finish` are normally transient within an API call, but explicit
mappings make capture total over the executor's states. The four fetch outcomes and
three delay outcomes above exhaust all current `prefetch_then`/`delay_then` call sites.
Reject other active continuation combinations instead of serializing an arbitrary
continuation tree. ([CPU execution][cpu].)

### Memory transfer fields

The existing `Transfer` already expresses hardware progress and can be reused privately
with explicit encoding; a duplicate generic instruction IR is not needed.

| Field | Retained meaning / validation |
| --- | --- |
| `address:u16` | Latched operand base, already even for word/long transfers. |
| `size:Size` | Byte, Word or Long. |
| `register:u8` | General-register alias field; 0–15 for byte/word, 0–7 for long. CCR transfer uses field 0. |
| `store:bool` | Direction; writes have already latched their entire source value. |
| `ccr:bool` | CCR load/store rather than MOV; requires Word and changes completion/flag behavior. |
| `absolute8:bool` | Absolute-8 MOV provenance, agreeing with the captured decoded addressing form. |
| `value:u32` | Entire pending store value, or already collected load data. A long load with `done=2` holds its first word in the low 16 bits. |
| `done:u8` | Bytes whose whole CPU-level transfer has completed: 0 for Byte/Word; 0 or 2 for Long. |
| `post:Option<(u8,u32)>` | Deferred full-ER post-increment destination/value; index <8. Apply only at operand completion, before destination-register replacement. |

For stores, bound `value` by the size mask; a CCR store's low byte is zero. For loads,
`value=0` before any word completes, and at most `FFFF` at long `done=2`. Match
size/register/direction/CCR/addressing-mode flags to the captured instruction without
recalculating a latched address or value from live registers. Pre-decrement has already
changed ER, including when that ER aliases the source. `post` is present only for the
corresponding load form. `DelayAfterAddressUpdate` requires `done=0` and a
pre-decrement/post-increment instruction. ([Transfer creation][cpu];
[memory decoding][decoder].)

### EEPMOV substeps

`CountWidth::Byte` maps to `word_count=false` and R4L; `Word` maps to true and R4. Both
forms transfer bytes. ER5/ER6 retain source/destination and their full 32-bit updates in
the common register bank.

| `CopyStep` | Live stage/value | Completed work / remaining action |
| --- | --- | --- |
| `ReadInitialSource` | 0 / 0 | Initial source dummy read remains. |
| `ReadInitialDestination` | 1 / 0 | Source dummy read done; destination dummy read remains, even with zero count. |
| `AdmitNextByte` | 2 / 0 | Between byte pairs; `.W` can admit NMI here. No source read has been issued. |
| `ReadAdmittedByte` | 4 / 0 | Admission is finished; an outstanding source read must remain stable if NMI arrives. |
| `WriteLatchedByte { value:u8 }` | 3 / value | Source byte captured; destination write remains. Pointers/count still describe this pair. |

Stages 2/3/4 require a nonzero count. Pointers increment and count decrements only after
destination-write completion; final NEXT fetch then uses `FetchNextBeforeRetirement`.
Stage 2 and 4 must never collapse into one saved "copying" state. ([EEPMOV
request/admission][cpu].)

## Issued request and physical lanes

Preserve `Machine.pending` separately from CPU progress. Its `Action` is the
already-issued logical request, while the lane and `ClockWait` describe the physical
action still in progress:

- `Read { address:u16, width:Byte|Word, fetch:bool }`.
- `Write { address:u16, width:Byte|Word, value:u16, mov_byte:bool }`.
- `Internal { states:u32 }`, mapping to `Action::Idle(states)`. This is the
  original interval length; the wait holds unfinished work.
- `wait`, `split:bool`, `lane:u8`, `high:u8` map to the existing `Pending`.
  `Action::Sleep` is never a pending timed action.

Validate `split == (width == Word && !Mcu::native_word(address & !1))`. Unsplit actions
and internal waits have lane 0. A split word has lane 0 or 1; lane 1 means the high-byte
access already happened. For a split read, `high` is that actual returned byte and
cannot be reread. For a split write, it is the already-written high byte (derivable from
`value`). Only the low lane is performed after restoration. `done` in a long `Transfer`
does not advance until both lanes of its current logical word finish.

Match a saved request to its suspended progress with a pure request projection,
sharing/factoring the existing action construction if useful. It must not call
`next(None)`: that can decrement deferral, consume prefetch, mutate registers or admit
EEPMOV's next pair. Request validation covers address/width, fetch flag, write value and
MOV.B provenance. Word bus addresses use `address & !1`; do not require arbitrary
stack/target input addresses to have been even. Preserve `instruction_address` and
`absolute8` so resumed MOV.B uses the same `WriteOrigin`.

Only suspended action-producing phases can have `pending`: reset/fetch, the fetch
variants, delays, memory/bit accesses, indirect/call/target/return/ exception accesses,
arithmetic interval, and EEPMOV stages 0/1/3/4. EEPMOV stage 2 cannot already have a
source request. Conversely, do not require every action-producing phase to have one:
initial construction, clock/reset holds, direct transitions and faults can leave work
not yet issued. ([Pending/Resume][machine].)

## Clock obligations and reconstruction

Save `ClockWait` as `{ source:ClockSource, divide:u32, obligation }`, where `obligation`
is `Running { target_edge:u64 }`, `Paused { remaining_edges:u64 }`, or `Ready { at:u128
}`. `at` uses the common 64.64-second timeline. Source names are System, Cpu, Watch,
OnChip, Oscillator and Subclock. Never convert an edge target into a duration from the
capture time or call `ClockWait::after` to reconstruct an existing Running obligation.

Omit `cached` and `revision`. Restore/validate clock authorities first, set a fresh
clock revision, then give every Running wait that revision and either the checked
`clocks.edge(target, tap)` projection when available or `Time::MAX` when unavailable. A
subsequent source change invalidates the cache normally. Do not initialize a supposedly
current cached timestamp to zero. Paused and Ready obligations need no cached
projection. ([ClockWait][clocks].)

Require a valid source and nonzero divider before any clock arithmetic. The CPU pending
wait uses Cpu÷1. Keep the other Machine obligations separate: delayed SLEEP uses Cpu÷1;
wake stabilization uses Oscillator÷1 and retains `direct`; watchdog reset uses OnChip÷1;
`reset_release` uses System÷1. `Resume::Sleep` means SLEEP has retired but its mode
transition is pending; `Resume::Wake` means the wake sequence is pending. Do not fold
these into CPU instruction progress. ([Machine reset][machine].)

For active available appointments, checked projection must succeed and give `deadline >=
Machine.now`; equality is valid because the horizon is exclusive. For unavailable
sources retain the obligation rather than inventing a current timestamp. Paused zero
remaining edges is legal. Validate the underlying clock/divider phases before attempting
projections; then rebuild scheduler appointments without consuming any edges or invoking
peripheral callbacks.

## Validation and disposable state

Decode into a candidate; reject before replacing the live machine. In addition to the
local bounds above:

- Require `fetched_count <= 5`, even architectural PC/instruction/prefetch
  addresses, and `interrupt_deferral <= 1`. Zero unused fetched slots during
  capture/reconstruction. Do not impose alignment on ER/SP or raw targets.
- Decode-required progress must have a complete matching instruction. A
  healthy `FetchInstructionWord` has fewer than five words and the decoder
  requests another word. Check instruction-class compatibility for transfer,
  bit, branch-calculation, EEPMOV and arithmetic progress; do not re-execute
  ALU or address-update effects to validate latched values.
- Arithmetic output size is Word with 12 states or Long with 20; destination
  field is <16/<8 respectively and result fits its size. Bit index is <8.
  Exception delay is 2 or 4. Retained flags are full `u8` values.
- Cross-check `exception_notification`, when present, with admitted entry
  progress, and retain it for normal integration to consume. Do not clear NMI
  or call flash protection during load. Do not force it to `None` merely
  because the usual queue path consumes it immediately.
- Respect terminal fault captures. On `Error::Decode`, `complete` has already
  incremented count/PC and retained the invalid word while phase remains
  `Fetch`. Bus/scheduling errors can also leave a phase without `pending`.
  Preserve the error latch and validate that particular stopped state;
  healthy-progress invariants must not reject snapshots the core produces.
  The normal run API must keep returning the saved error rather than resume
  the invalid instruction. ([Fetch completion][cpu];
  [fault latch][machine].)

Discard inactive `continuation`; inactive EEPMOV `value` outside stage 3; `Call.target`
after target fetch; `JumpTarget.return_pc` when no call follows; and pending `high`
before any first lane completes. Reconstruct canonical zeros for these. Keep active
partial-load/store values, saved exception CCR/PC, branch decisions, prefetched bytes,
and deferred register updates. `retired` and `interrupt_entries` are diagnostic
counters, not hardware authority; initialize them under the codec's stated diagnostic
policy. Consequently, restoration is inverse over causal progress, not bitwise equality
of stale scratch fields or profiler totals.

Capture and reconstruction use exhaustive matches beside the CPU owner; keep ordinary
`next`/`complete` as the only executor. The meaningful regression checks are identical
subsequent bus effects and timing after a capture between longword halves, between split
SFR lanes, during each exception stack phase, during EEPMOV admission/read/write, and
before arithmetic/CCR commitment. Encoded round-trip equality alone does not establish
it.

[cpu]: ../crates/hs-core/src/cpu/mod.rs
[decoder]: ../crates/hs-core/src/cpu/decode.rs
[machine]: ../crates/hs-core/src/machine.rs
[clocks]: ../crates/hs-core/src/mcu/clocks.rs
