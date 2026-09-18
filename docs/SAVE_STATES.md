# Native save states

A save state captures the running device at its exclusive time horizon. It
includes current flash, RAM, EEPROM, sensor nonvolatile cells, physical inputs,
clock phases, power retention, and unfinished CPU/device work. An EEPROM save
is just the persistent image used by firmware; loading one cannot resume a CPU.

`Machine::snapshot()` captures without advancing time or touching guest registers.
`Snapshot::encode()` writes the native file representation;
`Snapshot::decode(bytes)` constructs and validates an isolated candidate.
`Machine::restore(&snapshot)` checks the original firmware identity and replaces
the complete machine only on success. `Machine::from_snapshot` starts another
instance from the captured identity and hardware state. Both restoration paths
start diagnostic instruction, interrupt, bus and serial counters at zero.

```rust
# fn example(machine: &mut hs_core::Machine) -> Result<(), hs_core::Error> {
let bytes = machine.snapshot().encode()?;
let saved = hs_core::Snapshot::decode(&bytes)?;
machine.restore(&saved)?;
# Ok(())
# }
```

The origin identity is SHA-256 of the original construction firmware. Guest
flash programming does not change it. A state includes the current flash cells
and partial programming charge, so restoring an earlier point does not require
the current flash image to match its earlier contents. Loading does not write
any host EEPROM file; the frontend owns that policy.

## File contract

The envelope is eight bytes `HSTEPST\0`, a little-endian `u32` payload length,
32 bytes of SHA-256 over the payload, then the payload. The checksum detects
corruption; it is not authentication. The file has no format version, schema
fingerprint, migration layer or cross-release compatibility promise.

The payload uses [Borsh](https://borsh.io/#specification): fixed-width
little-endian integers, declaration-order fields, fixed arrays without lengths,
and explicitly assigned byte tags. Time is unsigned 64.64 seconds (`u128`),
source-edge ordinals are `u64`, and guest memory is in increasing address order.
No host `usize`, pointer, path, string or arbitrary collection is serialized.
The sole variable-size record is a bounded list of up to 384 touched internal
flash pages; each has a 16-bit aligned address and 1024 cell-charge values.

The maximum payload is 4 MiB, accommodating every flash page plus fixed memory
and owner records. Counts are checked before allocation; the large flash/EEPROM
byte arrays are read directly into fixed-size heap buffers. Frontends can use
`Snapshot::MAX_ENCODED_SIZE` to bound file reads. Decoding rejects trailing bytes,
bad checksums/tags, invalid indices/divisors/progress and missed active
appointments. Deliberately odd but reachable guest register settings remain valid.

The authoritative field declarations are the private
[`Saved` record](../crates/hs-core/src/machine/state.rs),
[CPU progress](../crates/hs-core/src/cpu/state.rs), and the component owners it
contains. Direct derives apply to hardware latches and histories. CPU progress
is mapped to named unfinished hardware work, independently of runtime `Phase`
ordering. See the [progress description](research/save-state-cpu-progress.md)
for each tag's completed effects and remaining action. This is HachiStep's native
format; agreeing on these semantics across emulators remains future work.

## What restoration preserves

A split word access retains its already-completed high byte; load does not repeat
that read or write. Longword halves, EEPMOV admission versus issued reads,
exception stack progress, latched arithmetic results and fetched instruction
words are retained. Decoded instruction objects are reconstructed from those
words, without reading memory or running the CPU.

Clock obligations retain their source, divider and target edge or paused edge
count. Projection caches and clock revision numbers are rebuilt. Lazy ADC
settling and Timer W capture-visibility waits may already be past, while an
active appointment at the exact exclusive horizon is still pending.

Flash/EEPROM/sensor programming retains original cells and its unfinished
operation, rather than replacing them with a projected persistent image. Sensor
filter sums are rebuilt from retained samples. LCD scan/output latches and GPIO
last-resolved levels remain causal: resolving the board on load would deliver
edges twice. No load callback or guest access occurs. A saved core fault remains
stopped, with a generic restored-fault diagnostic rather than the original text.

Profiler totals, decoded instructions, calendar caches, trace histories and
rendered frames are absent. Typed snapshots can retain disposable working
caches for cheap in-memory copies; their structural equality is useful within
the same execution history. Compare encoded causal records or subsequent
observations across file restoration. Hosts separately retain their future
input cursor and output-delivery position.

## CLI

```sh
hachistep run --firmware ROM --eeprom SAVE --milliseconds 1000 \
  --save-state session.state
hachistep run --load-state session.state --milliseconds 2000 --out resumed
```

`--out` also writes `state.bin`; all files use create-new semantics. Restoring
from a file supplies the whole machine, so image/initial-condition arguments
conflict with `--load-state`. Horizons and input CSV times remain absolute device
time; CSV entries before the restored instant are skipped. Outputs and reports
contain only the resumed segment's diagnostic/event totals.

Validation covers quarter-cycle CPU captures, split SFR lanes, copy and exception
progress, flash pulses, EEPROM interruption, clock/reset/power transitions,
private retail replay, malformed candidates and continuation without ordinary-run
allocation. Debug and release decoding are tested with a 1 MiB thread stack.
