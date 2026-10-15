# Save states

A save state resumes the complete device, including unfinished CPU accesses, clock
phases, physical inputs, and programming operations. An EEPROM image starts a new
session with those persistent bytes.

## CLI

```sh
hachistep run --firmware firmware.bin --eeprom eeprom.bin \
  --milliseconds 1000 --save-state session.state
hachistep run --load-state session.state --milliseconds 2000 --out out/resumed
```

`--out` also writes `state.bin`. Horizons and CSV timestamps stay absolute after load.
The second command runs the interval from 1000 to 2000 ms. Image and initial-condition
options conflict with `--load-state` because the state already contains them.

## Embedding

```rust,ignore
let bytes = machine.snapshot().encode()?;
let saved = hs_core::Snapshot::decode(&bytes)?;
machine.restore(&saved)?;
```

`restore` requires the same original firmware identity and leaves the machine intact
on failure. `Machine::from_snapshot` constructs another instance from a snapshot.
Guest flash programming does not change the original identity.

Restoration resets diagnostic counters. The host retains its input cursor, delivered
events, and audio playback queue separately. Recreate the audio renderer after load.
A captured fault remains stopped; restore a healthy checkpoint to resume execution.

## Storage

Native files are specific to the core revision. Keep the matching build with a saved
session. `Snapshot::MAX_ENCODED_SIZE` bounds reads. Decoding checks the payload length,
SHA-256 checksum, and hardware-state constraints before restoration.

The envelope is `HSTEPST\0`, a little-endian `u32` payload length, a 32-byte SHA-256,
and a Borsh payload. The maximum payload is 4 MiB. The checksum detects corruption;
applications accepting untrusted uploads must provide their own authentication.
