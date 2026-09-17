# Emulator saves and independent hardware tests

Research checked 2026-09-16 against current upstream source. This is a focused
comparison of concrete implementations and hardware suites. Implementing
cross-emulator save-state interchange remains deferred; the current state
description should support eventual convergence without promising it already.

## Saves and save states

The familiar distinction applies directly: a save preserves nonvolatile device
contents; a save state preserves the running machine so execution can resume.
SameBoy exposes separate battery-save and state APIs, with file and memory-buffer
forms. Battery saves include applicable cartridge RAM and RTC information.
Its state API calls the captured machine a snapshot.
([Battery API](https://github.com/LIJI32/SameBoy/wiki/GB_save_battery),
[state API](https://github.com/LIJI32/SameBoy/wiki/GB_save_state),
[API index](https://github.com/LIJI32/SameBoy/wiki))

The starter's `Snapshot` already implements the in-memory form of a save state:
`snapshot()` clones the machine, and `restore()` replaces it with that copy.
There is no serialized file format yet. This is a representation distinction,
not a third user-facing persistence category.
([Current implementation](../../crates/hs-core/src/machine.rs))

An EEPROM image is the ordinary Pokéwalker save. It is not every persistent
property of the hardware: the starter also exposes persistent EEPROM status and
sensor nonvolatile bytes. Those remain hardware state without requiring a new
frontend concept.
([Current persistence accessors](../../crates/hs-core/src/machine.rs))

## Useful mechanisms in established implementations

### SameBoy: reuse state capture for several consumers

SameBoy's native state stores named sections for CPU/core, DMA, cartridge,
memory, timing, audio, RTC, video, and accessories. Its file and buffer entry
points share the same serializer. Native loading reads fixed sections into a
temporary object, checks model and memory sizes, then restores memory and
rebuilds derived palette information. It also resets transient audio-output
state. This is not fully transactional loading: RAM buffers are read into the
live instance before the final object assignment.
([Save and restore implementation](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/save_state.c#L552-L594),
[load and fixups](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/save_state.c#L1292-L1355))

Rewind consumes the same native state buffers. It retains periodic complete
states and encodes intervening states as unchanged runs plus changed bytes
relative to the complete state. Rewind therefore does not need another hardware
execution mechanism. This is a useful extension point, not a reason to implement
rewind or compression in HachiStep immediately.
([Rewind implementation](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/rewind.c#L9-L194))

### mGBA: preserve pending work, rebuild derived scheduling

mGBA's GBA state includes fetched CPU material and pending interrupt timing.
Restore clears the timing queue, restores component state, and reconstructs
scheduled work. EEPROM/flash state separately retains the current command,
remaining read bits, addresses, programming state, and pending completion time.
Capturing registers and memory alone would miss this work.
([Machine serialization](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/src/gba/serialize.c#L27-L242),
[storage-operation serialization](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/src/gba/savedata.c#L715-L757))

mGBA can attach persistent save bytes to a state. Loading those bytes into the
emulated machine and writing them back to the host save file are separate
operations. Its GBA implementation supports immediate writeback or a temporary
save-data mask, with later writeback when save data is synchronized. This avoids
equating loading a state with immediately overwriting the ordinary save file.
([State attachments](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/src/core/serialize.c#L431-L442),
[restore policy](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/src/gba/core.c#L1493-L1506),
[mask handling](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/src/gba/savedata.c#L107-L135),
[eventual synchronization](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/src/gba/savedata.c#L590-L603))

### Dolphin: one field traversal, then separate expensive I/O

Dolphin's `PointerWrap` drives read, write, measure, and verification modes
through the same state traversal. Components enumerate their state once using
`DoState`. The useful principle is avoiding independently maintained save/load
field lists; the C++ wrapper itself is not a proposed dependency.
([Serialization wrapper](https://github.com/dolphin-emu/dolphin/blob/ee018d00e60b9eb727489908a8daec5c537f44a8/Source/Core/Common/ChunkFile.h#L36-L64),
[component traversal](https://github.com/dolphin-emu/dolphin/blob/ee018d00e60b9eb727489908a8daec5c537f44a8/Source/Core/Core/State.cpp#L138-L201))

Dolphin captures on the CPU thread, then hands the completed buffer to a worker
for compression and writing. It writes a temporary file before moving it into
place and keeps an overwritten-state backup. Its source explicitly identifies
remaining atomicity limitations, so this is evidence for the separation and
recovery pattern, not a guarantee that every write path is atomic. Before loading,
it captures an undo state; a failed load restores that state because loading may
already have changed the machine.
([Capture and file output](https://github.com/dolphin-emu/dolphin/blob/ee018d00e60b9eb727489908a8daec5c537f44a8/Source/Core/Core/State.cpp#L392-L505),
[failed-load recovery](https://github.com/dolphin-emu/dolphin/blob/ee018d00e60b9eb727489908a8daec5c537f44a8/Source/Core/Core/State.cpp#L810-L865))

### Near's serialization analysis: capture must preserve execution

Near describes how advancing or desynchronizing emulated components to reach
convenient serialization points can change later results, particularly under
repeated captures for rewind. The useful requirement for HachiStep is a capture
that preserves time and pending work. Its explicit CPU/peripheral continuation
state already provides the appropriate foundation; it does not need native-stack
serialization or a second execution mode.
([Cooperative serialization](https://archive.ares-emu.net/near.sh/articles/design/cooperative-serialization.html))

### BESS: portable restoration has an explicit fidelity contract

BESS appends implementation-independent information to an emulator's native
state. Its stated purpose is to let the native representation retain detailed
timing while the portable representation restores a session adequately for
ordinary use. This explicitly distinguishes best-effort transfer from exact
continuation; the two uses can share one file without offering identical
guarantees. ([BESS motivation](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/BESS.md#motivation))

For HachiStep, exact native restoration remains the requirement. A future shared
format must settle its fidelity contract explicitly. Documenting hardware state,
timing references, and operations in progress now gives that work a useful basis,
but neither common byte encoding nor hardware-oriented field names prove that
different emulators will resume identically. No BESS-style fallback format is
being selected now.

## Recommendation for HachiStep

These are design recommendations derived from those mechanisms and the current
requirements, rather than claims that all three projects make identical choices.

- Keep ordinary EEPROM saves and full save states. `Snapshot` is the current
  in-memory save state. A serialized state is an encoding of the captured machine.
- Capture at the core's stopped API boundary, preserving any CPU or peripheral
  operation already in progress. Do not advance the emulated machine merely to
  make capture easier.
- Include every value needed to continue the captured hardware: memory,
  nonvolatile contents, clock phases, fetched material, latches, and pending
  operations. Keep host handles, diagnostics, and rebuildable caches outside
  the authoritative state. Rebuild derived scheduling and presentation after
  restoration without causing guest accesses or advancing time.
- Define saved fields by their hardware meaning before selecting the codec.
  Cover completed effects, retained operands/results, operation progress, and
  clock obligations. Private continuation numbers must map to that description.
  Derive directly where the types already express the contract; otherwise use
  explicit save/load mappings, generated where useful. Share field definitions
  without turning every internal field into part of the saved contract. Use
  explicit widths and encoding. Serialization should add no work to ordinary
  emulation when no state is being captured.
- Decode an external state into a candidate, validate its lengths, device
  invariants, and required image contents, then replace the live machine. A rejected
  state must leave the current session intact. HachiStep's small machine makes
  this simpler than mutating it and maintaining a rollback mechanism.
- Keep compression, filenames, save slots, file replacement, and backup policy
  in the host layer. The core captures/restores state and exposes committed
  persistent storage. Background compression is worthwhile only when measured
  capture or I/O latency requires it.
- Full restoration rewinds emulated EEPROM as well as CPU/RAM. The frontend's
  ordinary save-file writeback policy is separate. Restoring old RAM against
  newer EEPROM would not restore the captured machine.
- Verify resumed behavior: capture during representative partial CPU, serial,
  timer, and storage operations; run a recorded input suffix; restore; replay;
  compare outputs and resulting machine state. A serialize/deserialize byte
  round trip alone cannot establish that nothing important was omitted.
- Add no save-state versioning, migration machinery, or compatibility promise
  before the project approaches 1.0. Mature projects' compatibility histories
  are not a reason to impose that work on the starter.

## Hardware test suites

These projects provide concrete precedents for a separately maintained hardware
corpus and a small emulator-side runner:

| Project | Observed organization |
| --- | --- |
| Mooneye | Separate test-ROM repository. Its documented research loop forms a hypothesis, runs a diagnostic on hardware, and revises the hypothesis. It distinguishes hardware acceptance, emulator-only, and manual tests. [Methodology](https://github.com/Gekkio/mooneye-test-suite/blob/31510e12eea6286d36eea060a6adde755e1067aa/README.markdown#hardware-testing). |
| SameBoy / SameSuite | SameSuite is a separate test-ROM repository intended to investigate unknown hardware behavior and verify emulation. SameBoy retains its runner/CI machinery in the emulator repository. [Suite](https://github.com/LIJI32/SameSuite/blob/f15645fb049a47ea235f6d2c9a033e72d8087901/README.md), [SameBoy CI runner](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/.github/actions/sanity_tests.sh). |
| mGBA | Its separate GBA suite runs as guest software, displaying results and writing them to SRAM. The emulator also has local core/API tests. [Suite](https://github.com/mgba-emu/suite/blob/e6942030d25ffe3ba76c72b73a86da073ec857cc/src/main.c), [API tests](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/src/gba/test/core.c). |
| Dolphin | Separate GameCube/Wii hardware tests with instructions for physical-console execution and collecting results. Local unit tests remain in Dolphin. [Hardware tests](https://github.com/dolphin-emu/hwtests/blob/f28077b139eec18967f60db6ce1e15b182dfeac0/Readme.md), [local timing tests](https://github.com/dolphin-emu/dolphin/blob/ee018d00e60b9eb727489908a8daec5c537f44a8/Source/UnitTests/Core/CoreTimingTest.cpp). |

Mooneye's timer-reload case is an example of an implementation-independent
contract: guest instructions write a timer at adjacent timings, read the result,
and compare architectural register values with expectations verified on hardware.
The test does not name an emulator's timer object, event queue, or helper calls.
[Test source](https://github.com/Gekkio/mooneye-test-suite/blob/31510e12eea6286d36eea060a6adde755e1067aa/acceptance/timer/tima_write_reloading.s).

For HachiStep, keep guest diagnostics, signal fixtures, independently justified
expectations, and hardware collection tools in the separate suite. Keep its
adapter and a small set of useful embedding/regression tests with the core.
Save/restore reproducing the next observable effects is a meaningful local
contract; matching a private object layout or the current implementation's own
generated answers is not a hardware specification.
