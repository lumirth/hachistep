# Native save-state codec contract

Research handoff, 2026-09-18, against the current working tree. This follows
[DESIGN §13](../DESIGN.md#13-public-api-and-save-states) and the existing
[emulator-state research](emulator-state-and-testing.md). It does not install
dependencies or implement a codec. `Snapshot` currently clones `Machine`;
there is no serialized format or immutable firmware-origin identity yet.
([Current snapshot implementation][machine])

## Recommended representation and API

Use Borsh for a private, typed **causal state record**. Reuse hardware-owner
types where their fields already describe the saved hardware. Supply explicit
adapters for CPU operation progress, clock waits, large boxed byte arrays and
cache reconstruction. A small top-level record selecting `Machine` fields is
appropriate; duplicating every device into parallel serializer-only structs is
not. Keep one field declaration driving both directions through derives or a
small declaration macro.

Proposed file layout:

```text
8 bytes   magic: HSTEPST followed by zero
u32 LE    payload byte length
32 bytes  SHA-256 of the complete payload
payload   Borsh-encoded causal state record
```

The checksum detects accidental corruption before constructing state; it is
not authentication. There is **no format version, schema hash, build identifier,
migration, compatibility switch or second restoration path**. This is the
current development contract. Field/tag changes update its description and
implementation together; files from other development builds are not promised
compatible or always distinguishable.

Useful Rust surface: `Snapshot::encode() -> Result<Vec<u8>, Error>`,
`Snapshot::decode(&[u8]) -> Result<Snapshot, Error>`, existing `snapshot()`,
`restore(&Snapshot) -> Result<(), Error>`, and `from_snapshot(&Snapshot)`.
Only the validated `Snapshot` is public; decoding must not expose an unchecked
`Machine` through a public derived deserializer. Frontends retain compression,
filenames, file replacement, save slots and EEPROM writeback policy.

Borsh supplies little-endian fixed-width integers, field-order serialization,
fixed arrays without a length prefix, and tagged options/enums. Memory arrays
remain bytes in increasing emulated address order; H8 guest endianness does not
change their byte ordering. Use only explicit integer widths in the record,
not `usize`, raw Rust layout or incidental live-enum ordinals. Assign documented
`u8` tags to hardware modes and progress states; derives can honor explicit
discriminants. No strings, maps, arbitrary vectors or recursive records are
needed for the current hardware state. ([Specification][borsh-spec],
[derive attributes][borsh-derive-doc])

## Exact state inventory

The payload starts with `firmware_origin: [u8;32]`, followed by timeline/board,
CPU, MCU and external-device records in a fixed declared order. The following
is the ownership checklist, including hidden state that register dumps miss.
Field widths follow the linked current owner definitions unless an adapter is
specified below.

| Owner | Retain |
| --- | --- |
| Machine timeline/conditions | `now`, `last_effect`; all `Conditions`; seven optional analog-pin voltages; power/reset flags; incident IR input; pending CPU access; sleep/wake continuation including `direct`; watchdog-reset clock obligation; terminal-fault latch |
| Board connections | GPIO configuration/output latches and external input levels; last resolved pad levels and sampled IRQ/NMI/serial levels; `Machine.serial`, emitted-IR and piezo baselines. These establish which physical edges have already been delivered. |
| CPU | All ER registers, PC, CCR, instruction address, fetched words/count, retained prefetch address/word, operation progress and latched operands/results, delayed interrupt admission, and an accepted vector awaiting controller acknowledgement |
| MCU memory/control | 49,152 flash bytes, 2,048 RAM bytes, `reset_held`; SYSCR/OSCCR/module gates, operating mode, `stabilizing_from`, feedback-cut latch, IRQ/NMI levels and pending requests |
| Clocks | Every source/domain's rational phase, edge ordinal, running/held state; selected routes/dividers; both shared prescalers' anchor, phase, emitted-edge counts and running state |
| RTC / B1 / WDT | Registers/counters, reload latch, consumed-edge ordinals, gates; RTC phase and pending calendar update; WDT private ROSC divider, overflow read qualification and reset/source state |
| Timer W / AEC | Registers, counters, outputs, read qualifications, consumed edges and synchronization epoch; Timer W delayed clear/capture visibility and three-stage pin pipeline; AEC input history, PWM phase and requests not yet transferred to the interrupt controller |
| ADC / comparators | Current conversion phase, held sample and result, trigger synchronizer/history and settling/conversion obligations; comparator result/baseline/target, arming/read qualification, due time and same-time interrupt suppression state |
| SSU / SCI | Holding and shift registers, receive/transmit progress, read-qualified flags, pins and edge history, clock source/divider progress, selected frame format, preloaded next character, IR pulse stretch/end and mux-glitch obligations. Exclude diagnostic byte counters. |
| M95512 | Original array/status, WEL/protection, selection history, command/address and serial latches, 128-byte page latch, written-bit mask, data count/pending status, active `WriteCycle { started, deadline }`, page base or status target, configured write duration |
| BMA150 | Working/nonvolatile registers, serial/shadow/turnaround latches, physical input and temperature, filter history/cursors/counts/window, last filtered samples, sample-clock phase, wake/pause/quiet/IRQ/image/NV obligations, sleep/self-test progress, interrupt detector histories/latches and data-ready state |
| NT7508 | Pixel/icon RAM, serial/command latches and register settings; scan-clock phase, latched frame geometry, row/PWM/FRC/inversion progress and segment-output latch |

Sources: [machine][machine], [CPU][cpu], [MCU composition][mcu],
[clocks][clocks], [MCU owners][mcu-directory], [external owners][devices].
Implementation update: the record now includes IIC2, internal flash pulses and
cell charge, the RES capacitor/retention owner, and independently qualified
source startup. See [the implemented contract](../SAVE_STATES.md).

**Do not use ordinary persistent-image accessors to capture state.**
`Machine::eeprom()`, `eeprom_status()` and `sensor_nonvolatile()` project partly
completed writes at `now`. Save the original cells **and** the retained write
cycle/target instead. Combining projected cells with the old cycle changes
subsequent interrupted-write behavior. The NT7508 scan's latched geometry and
segments likewise cannot be reconstructed from current display RAM after
firmware changed RAM within the same scan interval. ([NV progression][nv],
[EEPROM owner][eeprom], [LCD scan][lcd-scan])

## CPU progress is hardware work, not a continuation number

Use a flat, tagged `CpuProgress` record with meanings below. This can map
directly to today's `Phase` variants during capture/load; it does not require
changing the executor or adding a universal instruction representation.

| Saved progress | Meaning and payload |
| --- | --- |
| Reset vector / boundary / instruction fetch | Reset-vector read outstanding; between instructions; or fetching the next word of the captured instruction. Preserve fetched words, not replacement reads from memory. |
| Preparation / execution-ready | Instruction bits are fully captured but the relevant setup/execution effects have not occurred. Reconstruct `Instruction` by decoding those captured words. |
| Prefetch / internal wait | Outstanding fetch address and retain/discard choice, or initial internal-state count; plus the semantic operation that follows. |
| Memory transfer | Base address, byte/word/long size, load/store/CCR and MOV.B-origin meaning, register field, latched write or partial read value, bytes already completed, and deferred post-increment register/value. Pre-decrement effects are already in ER. |
| Memory bit operation | Read address and bit operation/index, or pending write address and already computed byte. Retain flags already changed by the read. |
| Branch/call/jump/return | Latched target/condition/return PC, whether target fetching or stack writing remains, and which stack word has already been read/written. Preserve SP's completed changes. |
| Exception entry/return | Accepted vector; saved PC/CCR; stack PC, stack CCR, vector or target-fetch stage; restored CCR pending the PC read; stabilization/internal-state obligation |
| Multiply/divide | Result and CCR already calculated but awaiting commitment, destination/width, and remaining clock obligation |
| EEPMOV | Byte/word count form; initial source/destination dummy reads, inter-pair interrupt-admission boundary, admitted source read, or destination write with its latched byte |
| Finish / sleep | Instruction effects complete but retirement pending, or suspended by SLEEP |

In particular, current EEPMOV `stage=2` permits NMI admission, while `stage=4`
means the next source read has already been admitted. They must not collapse to
one “copying” state. Current continuation targets are bounded alternatives
(`Execute`, `Finish`, `ExceptionPc`, `BitWrite`, `Memory`), not recursively nested
continuations. Capture one active continuation only; discard an inactive stale
`Cpu.continuation`. ([`Cpu::next`, `complete`, and continuation helpers][cpu])

The issued physical access is separately described by current `Pending`:
read/write/internal wait, address, width, fetched-versus-data read, write value
and MOV.B provenance; plus its `ClockWait`, split flag, lane and retained high
byte. `lane=1` means the first byte-wide SFR access **already happened**, even
though the CPU-level word transfer is incomplete. A long transfer additionally
tracks completed word bytes in CPU progress. Restore neither repeats that first
lane nor prevalidates an entire instruction as one atomic access.
([`Pending` and `complete_cpu`][machine])

Only `Instruction` interpretation is rebuildable. Fetched bits, prefetch,
latched results and post-update values remain causal even when RAM/flash now
contains different bytes. Pure decoding during load must not call `Cpu::next`,
which can admit an exception or commit setup effects.

## Time, clocks and omitted caches

All `Time` and `Duration` values are unsigned **64.64 fixed-point seconds**,
encoded as `u128`; ordinals/counts are `u64`. `now` is the exclusive unprocessed
run horizon. An appointment equal to `now` is valid and remains pending.
`last_effect` records the last committed effect boundary and is needed by the
current projection APIs; neither timestamp is advanced for capture/load.
([Time types][time], [run loop and projections][machine])

For each `Clock`, retain `at`, `whole`, `remainder`, `denominator`, `fraction`
and `ordinal` with this meaning: the interval for `n` more edges is
`n*whole + floor((fraction+n*remainder)/denominator)`. Here `denominator` is the
fractional-period divisor, not necessarily the user-facing frequency
denominator. A `Domain` adds `running` and `held_at`. Prescaler output ordinals
do not reduce to oscillator ordinal divided by the divider after resets;
retain `anchor`, `phase` and `emitted[]`. ([Clock arithmetic][time],
[domain][clock-domain], [prescalers][prescalers])

Encode each clock obligation as a source/divider plus exactly one of:

- `Running { target_edge: u64 }`: outstanding source-edge ordinal;
- `Paused { remaining_edges: u64 }`: unconsumed work while a downstream gate is closed;
- `Ready { at: Time }`: an already-ready obligation awaiting its effect boundary.

Do not convert running targets to wall-time durations: a later clock switch
must change their projection. Do not turn paused remaining edges into an old
wall-time remainder. ([`ClockWait`][clocks])

Omit and rebuild:

- `Machine.next_devices`; recompute it from owner obligations after validation.
- `Clocks.revision` and `ClockWait::Running.cached/revision`; reinitialize the
  cache epoch and rebuild projections. **Defaulting all three to zero is wrong:**
  a zero cached timestamp would falsely appear current. Leave projections
  explicitly invalid until reconstructed against the restored clocks.
- Decoded `Instruction` values and inactive continuation scratch.
- BMA150 `Filter.sum`; recompute each axis from its saved history, cursor,
  count and window. Calling `select()` with the unchanged bandwidth will not
  rebuild it because that method returns early. Keep the last published
  `filtered` values and shadow latches. ([Filter implementation][filter])
- CPU retired/interrupt counters, Machine statistics, SCI/SSU byte counters,
  rendered pixels, trace output and other host diagnostics.

Retain board connection/edge baselines in the first codec. Some pad-drive fields
duplicate device outputs, but they record the last resolved instant, potentially
before an event exactly at `now`. They are not permission to resample physical
inputs during load. `resolve_board()` invokes chip-select handlers, serial clock
edges and interrupt sampling; **it is not a reconstruction helper**. Pure
presentation/net reconstruction must preserve that epoch and emit no events.
The mature precedent is restoring causal component state and reconstructing
appointments, as mGBA does for its CPU and storage operations—not serializing
its queue as a second authority. ([mGBA machine restoration][mgba-state],
[storage-operation state][mgba-storage])

`Machine.fault` also controls future execution. Do not silently skip it. For the
first codec, retain a `stopped_by_core_fault` boolean and restore a fixed
`Error::Snapshot("saved session was stopped by a core fault")` when set.
Static diagnostic strings and their exact wording are not hardware state;
the restored session must nevertheless remain stopped. Apply the documented
diagnostic-counter reset policy consistently to typed and encoded restoration.

## Firmware identity and full restoration

Compute SHA-256 of the original 49,152 firmware bytes once at construction;
retain that immutable identity in Machine/Snapshot and the payload. Restore into
an existing machine requires the same origin identity. Check this **before**
replacement; a mismatch leaves the live session untouched. Creating a new
machine from a decoded snapshot is explicitly allowed to use its captured
identity and full captured flash contents.

The origin identity is not the digest of flash *at capture*: custom firmware
can legitimately program internal flash. Comparing the latter would prevent
restoring a previous instant of the same device. The format includes all current
flash, EEPROM/status and sensor nonvolatile data; it does not mix captured RAM
with the host's newer EEPROM. A frontend can show the identity without exposing
the firmware bytes. Host save-file writeback remains a separate decision.

## Bounded decode and atomic installation

Implement this order:

1. Check magic, exact outer length and a **4 MiB current-format cap** before
   hashing or decoding. Fixed arrays total about 119 KiB; all 384 touched flash pages add about 3 MiB.
   Remaining records are bounded scalar/fixed-array state. Derive/check a schema maximum
   against this cap; a future flash-exposure representation must increase it
   deliberately according to its hardware bound, not accept arbitrary vectors.
2. Check the payload checksum. Decode from that exact slice and reject trailing
   bytes. Borsh's `from_slice` already enforces complete consumption.
   ([Implementation][borsh-complete])
3. Decode large boxed byte arrays into exactly their compile-time lengths using
   one safe heap helper (`Vec` of known length → `Box<[u8;N]>`, `read_exact`).
   Do not deserialize a file-supplied length and then reject it after allocation.
   Default Borsh boxed-array decoding constructs the owned fixed array first;
   a custom `deserialize_with` avoids large stack temporaries for flash/EEPROM.
4. Validate local representation bounds **before** invoking arithmetic or
   projections: tags, booleans, array indices, shift counts, divisors, clock
   fractions, register storage masks, transfer stages and payload relationships.
5. Restore clock routes/phases, rebuild clock-wait projections, validate
   cross-owner state, rebuild only proven caches and compute the next calendar
   entry. Do all of this on the candidate without guest accesses, event output,
   oscillator restarts or clock synchronization to `now`.
6. Check the destination's immutable firmware identity and swap in the complete
   candidate. Any error before this step leaves the live machine and host saves
   untouched. No rollback copy of a partially mutated live machine is needed.

([Complete-input and boxed-array implementation][borsh-de],
[custom field deserializers][borsh-derive-doc])

Concrete validation requirements:

- CPU fetch count ≤5; register selectors valid for their widths; transfer
  `done` matches completed byte/word granularity and is below the total size;
  pending lane is 0 or 1, with lane 1 only for a split word. Validate the issued
  request against saved semantic progress **without admitting new CPU work**.
- Clock divisor/denominator and period are positive; fraction/remainder are
  below their divisor; period multiplication/addition is checked before calling
  existing arithmetic. Prescaler phase fits its width. Validate source-specific
  ordinals and waits against restored domains, including halted domains.
- `last_effect <= now`; actual pending appointments cannot project before
  `now`, but may equal it. Historical sample/phase timestamps and lazy owners
  may be older: do not reject them merely for not being synchronized to `now`.
- Serial bit positions, frame lengths, LCD row/PWM/FRC indices, filter cursors
  and history counts fit the arrays/shift operations they index. LCD scan
  settings need not equal newly written display registers: they latch at
  different hardware boundaries.
- M95512 page base is aligned and within the array; sensor NV address is
  `2B–3D`; a write cycle has `started <= now <= deadline` at a stopped active
  operation. Preserve pre-operation data, targets and written masks together.
- Validate what the owners can actually retain, not an idealized legal
  firmware configuration. For example, malformed BCD RTC digits are reachable;
  rejecting every second/minute value above 59 would reject valid captures.

The loader's validation contract protects the runtime assumptions of these
types. It does not attempt to prove that every register combination could have
been reached by retail firmware or normalize guest-visible state.

## Dependency choice at the declared Rust 1.74 floor

The earlier note preferred Borsh 1.8.1, which declares Rust 1.77. The current
workspace still declares **1.74**. To implement this contract without changing
that floor, use:

```toml
borsh = { version = "=1.5.7", default-features = false, features = ["std", "derive"] }
sha2 = { version = "=0.10.9", default-features = false, features = ["std", "force-soft"] }
```

Borsh 1.5.7 and its derive crate declare Rust 1.67 and support the needed
integer/array/skip/custom-field functionality. Its derive graph permits newer
`proc-macro-crate` releases; **lock `proc-macro-crate` to 3.2.0 and `indexmap`
to 2.7.1** for the 1.74 build. Their published minimums are 1.67 and 1.63;
`toml_edit` 0.22.27 is 1.66. Checked current `syn` 2.0.118 and `quote` 1.0.46
declare 1.71, `proc-macro2` 1.0.106 declares 1.68, and `once_cell` 1.21.3 is
1.65. Keep these as resolved lockfile constraints, not unnecessary direct
runtime dependencies. ([Pinned Borsh manifests][borsh-manifest],
[derive graph][borsh-derive-manifest], [proc-macro-crate metadata][pmc],
[indexmap metadata][indexmap])

`sha2` 0.10.9 documents Rust 1.41 and supplies both image identity and checksum;
`force-soft` selects its portable software implementation. Hash only during
construction/capture/load, never ordinary execution. Share this dependency with
the CLI when replacing its handwritten SHA-256 rather than adding another hash
implementation. No Serde, Borsh schema-generation, compression or new test
dependency is required by the codec. ([SHA-2 package README][sha-readme],
[package features/API][sha-api])

These package declarations and transitive metadata were checked, not built.
The implementation must verify the final **locked graph** with Rust 1.74;
crate-level MSRV declarations alone do not prove that graph. If the already
discussed compiler-floor update is taken instead, update the manifest honestly
and use the current Borsh candidate rather than carrying old-version pins.

## Implementation acceptance

Require resumed observable behavior to match after capture within a split SFR
access, long transfer, exception entry, EEPMOV pair, gated clock wait, SCI/SSU
frame, RTC busy update, ADC sample/conversion gap, sensor filter/history state,
LCD row, and EEPROM/NV write. Include events exactly at the captured horizon
and identical subsequent input suffixes. Corrupt/truncate/checksum-mismatch,
invalid-index and wrong-origin loads must leave the existing machine unchanged.
Byte round trips supplement these checks; they do not establish causal coverage.

[machine]: ../../crates/hs-core/src/machine.rs
[cpu]: ../../crates/hs-core/src/cpu/mod.rs
[mcu]: ../../crates/hs-core/src/mcu/mod.rs
[mcu-directory]: ../../crates/hs-core/src/mcu/
[devices]: ../../crates/hs-core/src/devices/
[clocks]: ../../crates/hs-core/src/mcu/clocks.rs
[clock-domain]: ../../crates/hs-core/src/mcu/clocks/domain.rs
[prescalers]: ../../crates/hs-core/src/mcu/clocks/prescaler.rs
[time]: ../../crates/hs-core/src/time.rs
[nv]: ../../crates/hs-core/src/devices/nv.rs
[eeprom]: ../../crates/hs-core/src/devices/m95512.rs
[lcd-scan]: ../../crates/hs-core/src/devices/nt7508/scan.rs
[filter]: ../../crates/hs-core/src/devices/bma150/filter.rs
[borsh-spec]: https://borsh.io/#specification
[borsh-de]: https://github.com/near/borsh-rs/blob/abb9582c70b2afd54eef302c23b6e6d3a0b2c1c4/borsh/src/de/mod.rs#L763-L833
[borsh-complete]: https://github.com/near/borsh-rs/blob/abb9582c70b2afd54eef302c23b6e6d3a0b2c1c4/borsh/src/de/mod.rs#L1033-L1046
[borsh-derive-doc]: https://docs.rs/borsh/1.5.7/borsh/derive.BorshDeserialize.html
[borsh-manifest]: https://github.com/near/borsh-rs/blob/abb9582c70b2afd54eef302c23b6e6d3a0b2c1c4/Cargo.toml
[borsh-derive-manifest]: https://github.com/near/borsh-rs/blob/abb9582c70b2afd54eef302c23b6e6d3a0b2c1c4/borsh-derive/Cargo.toml
[pmc]: https://crates.io/api/v1/crates/proc-macro-crate/3.2.0
[indexmap]: https://crates.io/api/v1/crates/indexmap/2.7.1
[sha-readme]: https://docs.rs/crate/sha2/0.10.9/source/README.md
[sha-api]: https://docs.rs/sha2/0.10.9/sha2/
[mgba-state]: https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/src/gba/serialize.c#L134-L242
[mgba-storage]: https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/src/gba/savedata.c#L715-L757
