# Emulator embedding APIs

HachiStep needs a small device API that serves interactive frontends, deterministic
replay, hardware diagnostics, and infrared environments. Established cores share
construction, execution, inputs, usable outputs, persistence, and debugging. Their
choices about frames, callbacks, files, and threads follow their consumers. A generic
multi-console interface would bring obligations that this single-device core does
not yet need.

This comparison examines primary source and actual callers at the revisions linked
below, checked on 2026-09-30. Observations describe those implementations. Recommendations
for HachiStep are inferences from those observations and the current
[design contract](../DESIGN.md#13-public-api-and-save-states). Existing notes own the
detailed comparisons of [linking](emulator-linking.md),
[presentation](emulator-presentation.md), [save states](emulator-state-and-testing.md),
and [dependencies](emulator-dependencies.md).

## Pokéwalker implementations

| Project | Observed API and caller | Consequence for HachiStep |
| --- | --- | --- |
| PocketWalker | `Start` runs a paced loop; the Qt application starts it on a worker thread. Buttons, EEPROM replacement, byte infrared callbacks, Watts and session-step edits are exposed directly. [Interface][pw-api], [execution][pw-run], [Qt consumer][pw-consumer]. | Frontends need convenient button input, persistence, output, and stopped edits. Its firmware-specific edit names do not fit HachiStep's arbitrary-firmware contract. Host pacing in `Start` also prevents using that entry point as a deterministic bounded runner. |
| PokeStroller | Global machine state has `initWalker`, `runNextInstruction`, `setKeys`, and `fillVideoBuffer`. The Windows main loop tracks cycles and calls `quarterRTCInterrupt` itself. Initialization reads fixed `rom.bin` and `eeprom.bin` paths. [Interface][ps-api], [caller][ps-consumer], [initialization][ps-init]. | A small declaration count can hide hardware work in the frontend. HachiStep should continue to own RTC advancement and accept image bytes, rather than require a caller to supply interrupts or files. |
| Powar | A `pw_context_t` contains the device state, but the execution and SDL presentation loop share `main.c`. Its browser creates ROM and EEPROM files in Emscripten's filesystem and starts the application. [Context][powar-context], [loop][powar-loop], [browser][powar-web]. | A browser build alone does not establish a reusable embedding API. The useful need is running the same device engine with browser-owned storage, input, presentation, and scheduling. |

PocketWalker's network caller accumulates transmit bytes for a configured 5 ms timeout,
sends them through TCP, and feeds received bytes into `ReceiveIR`. Its SCI receiver
queues whole bytes until the receive register is free. These are practical application
mechanisms, but they omit optical levels and arrival timestamps. They cannot represent
malformed pulses or framing and timing-dependent overrun. HachiStep's timestamped
infrared input/output must remain the hardware boundary. A compatibility adapter can
live outside the core when a consumer requires one. [Network][pw-network],
[SCI][pw-sci], [HachiStep connection contract](../DESIGN.md#135-execution-pacing-and-external-connections).

These source inspections do not rank the projects' hardware accuracy or performance.
The interfaces show which work their existing applications perform.

## Established and Rust cores

| Project and context | Observed division of work |
| --- | --- |
| SameBoy, Game Boy library and desktop applications | An instance pointer identifies the machine. `GB_run` returns executed ticks; `GB_run_frame` runs to a vertical blank with caller-controlled pacing. The API offers memory-buffer loading, battery data, pixels, callbacks, registers, and direct memory access. Its Cocoa linked caller interleaves two instances using their executed ticks. [API][sb-api], [execution][sb-execution], [linked consumer][sb-link]. |
| mGBA, Game Boy/GBA library with Qt and SDL callers | `mCore` provides construction/loading, frame/loop/instruction execution, video buffers, audio, keys, state buffers, memory operations, and optional debugger attachment. The SDL application uses the separate `mCoreThread` integration, which invokes the same `runLoop`. [Interface][mgba-api], [SDL caller][mgba-consumer], [thread runner][mgba-thread]. |
| melonDS, Nintendo DS/DSi core with Qt/SDL caller | `NDS` owns the console and exposes frame execution, keys, and state operations. `Platform` supplies storage, host services, and timestamped multiplayer packets. The Qt emulation thread sets input, runs the frame, and performs host pacing/audio synchronization. [Console][melon-api], [platform][melon-platform], [caller][melon-consumer]. |
| jgenesis, Rust multi-console backends with native and browser callers | A shared `EmulatorTrait` receives renderer, audio, input-poller, and save-writer services. Backends return `TickEffect`; native and web callers repeatedly tick until a frame completes. Each output service has an associated error type. [Traits][jg-traits], [Game Boy backend][jg-gb], [native runner][jg-native], [web caller][jg-web]. |
| Boytacean, Rust Game Boy core with Wasm and libretro consumers | `GameBoy` offers instruction, cycle-budget, and frame execution; button operations; borrowed native buffers; and copied Wasm results. Its libretro adapter batches CPU work, exports video/audio, and serializes through the core state manager. [Execution][bt-api], [buffers][bt-buffers], [adapter][bt-retro]. |
| libretro, C contract spanning systems and frontend platforms | `retro_run` executes one frame and calls supplied input/video/audio callbacks. States use caller-supplied buffers; persistence is separately exposed as memory. The contract is single-instance and allows global state. [Run/state contract][lr-run], [memory contract][lr-memory], [official development guide][lr-guide]. |

## Execution and input timing

Instruction stepping, cycle budgets, and frame runs are useful conveniences, but they
promise different stopping behavior. Boytacean's `clocks_cycles` keeps stepping while
the total is below the requested limit, so the last instruction can cross it. Its
libretro caller retains excess cycles for the next call. `next_frame` waits for a PPU
frame change. [Budget execution][bt-api], [caller remainder][bt-retro].

HachiStep's exclusive time horizon is a stronger fit for physical inputs and linking.
The LCD may be stopped, and infrared work can occur between presentation frames.
Preserve `run_until` as the execution operation; frontend frame cadence can select its
horizons. An instruction or breakpoint operation, when required, should use that same
executor and state its treatment of partially completed instructions.
[Current execution contract](../API.md#advance-and-input-consumption).

Common frontend input APIs set the current buttons or poll a current input object.
jgenesis's Game Boy backend polls before each instruction; its browser caller supplies
a constant input object for a frame. mGBA exposes current key masks, and its GBA key
operations also check keypad interrupts. [Input traits][jg-traits], [backend][jg-gb],
[browser][jg-web], [GBA keys][mgba-keys].

HachiStep needs timestamped physical changes for both live input and replay. Current
button positions are sufficient at the host UI boundary; the application maps them to
machine timestamps. The same representation must support independent changes at one
instant, reject contradictory assignments before mutation, and report the consumed
prefix after an early return. Keep validation proportional to the supplied interval.
These requirements come from the
[input contract](../DESIGN.md#132-inputs), not from other emulators' untimed setters.

The current `validate_inputs` checks the entire supplied slice before execution, even
when some events are beyond the horizon. The documented caller partitions its input
first. That bounds work for ordinary runs, but repeated output stops can still leave
the same validated tail to scan again. [Validation](../../crates/hs-core/src/machine.rs),
[caller contract](../API.md#advance-and-input-consumption).

If that cost or caller complexity matters, separate immutable timeline validation
from machine advancement. A borrowed validated timeline can prove ordering,
same-time consistency, and value bounds once; a cursor then identifies the remaining
prefix without allocating or retaining history in `Machine`. Each advance still checks
the machine-relative horizon and rejects past input. A new live batch needs validation
before its first use. Never let slicing or cursor advancement split a same-time batch.
This is one input-consumption contract for replay and live adapters, rather than a
second execution path. Measure it with early-return/link callers before selecting an
additional public type.

## Outputs, callbacks, and errors

Usable pixels and sampled audio are common outputs. SameBoy and mGBA accept destination
buffers; jgenesis passes frames to a renderer and audio to an output trait. HachiStep
already provides requested pixels and reusable PCM conversion. Keep controller
behavior in the device and playback/window services in callers. Display tint and audio
queue policy do not require more hardware API. The detailed evidence and responsibility
choice remain in [presentation research](emulator-presentation.md).

jgenesis propagates renderer/audio/save failures through typed error variants. Its
Game Boy tick can complete a CPU instruction before audio or presentation fails.
This mechanism communicates the host failure; it does not make the tick transactional.
[Error variants][jg-errors], [tick implementation][jg-gb]. HachiStep's sink can retain a
typed host error and request return through `ControlFlow`. That avoids requiring the
core's error type to know about files, GPUs, or audio devices. Keep the synchronous
borrowed sink and explicitly document that a return request finishes the timestamp,
including further outputs. A callback failure therefore cannot retract delivered
effects. [Current sink contract](../API.md#advance-and-input-consumption).

Before freezing, give `RunResult` an explicit reason for returning, such as the
requested horizon or an output stop request. A stop request can coincide with the
requested horizon, so comparing times cannot always distinguish the two. Keep host
stop state outside snapshots. A fault should remain distinct from a successful early
return; its error needs the completed/failed position and a documented recovery path.
[Current result and fault behavior](../API.md#advance-and-input-consumption).

Input and state errors also need machine-readable categories when callers must choose
a remedy. The current `BadInput(&'static str)` and `Snapshot(&'static str)` variants
group multiple causes into prose. Prefer a small set of typed causes for errors a
consumer actually handles, with `Display` for people. Do not require callers to parse
error text, and do not turn every internal assertion into a public variant.
[Current errors](../../crates/hs-core/src/error.rs).

## Persistence and stopped edits

mGBA distinguishes battery-save data from running state and exposes a `writeback`
choice when restoring save data. melonDS calls the platform with the changed save or
firmware range and the full persistent data. jgenesis delegates writing to `SaveWriter`
and decides when to persist dirty cartridge memory in its backend. These examples
show several workable policies; none requires HachiStep to open a host save file.
[mGBA persistence API][mgba-api], [melonDS persistence][melon-platform],
[jgenesis persistence][jg-gb].

HachiStep should expose persistent bytes/status and hardware commit/interruption
events, while the application owns file replacement, debounce, backup, and the policy
after loading a save state. A host may be unable to write an EEPROM file immediately;
that must not make a successfully completed device write disappear. The running state
and export contract remain in [save state research](emulator-state-and-testing.md)
and [native save states](../SAVE_STATES.md).

HachiStep's `Images` groups firmware, EEPROM bytes/status and optional sensor
nonvolatile bytes. Callers supply the complete persistent device contents in one
description, and construction copies their borrowed buffers. These images start a
new session with the sensor's working registers loaded from its persistent image.
They do not replace a snapshot that retains unfinished hardware work.
[Current construction](../API.md#construction).

General stopped edits address the user needs behind PocketWalker's Watts and steps
operations without embedding those firmware meanings. mGBA provides separate bus and
raw memory functions: GBA bus writes call ordinary stores, while raw writes call patch
operations. SameBoy's direct-access API also warns that some I/O requires normal
memory operations. [GBA implementation][mgba-memory], [SameBoy direct access][sb-api].

Preserve the distinction in HachiStep. A direct RAM/EEPROM edit advances no clocks and
does not impersonate guest bus traffic. Every supported storage domain needs explicit
rules for unfinished work, invalidation, and persistence events. Add domains or
guest-access operations for concrete consumers; arbitrary access to private owners is
not a substitute. [Current edits](../API.md),
[design](../DESIGN.md#131-public-interface).

## Debugging and the public boundary

An embedding API should not accidentally freeze the CPU executor or peripheral
implementation. mGBA presents registers, memory blocks, and debugger attachment through
the core contract. SameBoy provides an extensive debugger API in addition to its
ordinary execution entry points. jgenesis's native runner selects a debugger process
that uses the same emulator and host services. [mGBA API][mgba-api],
[SameBoy debugger][sb-debugger], [jgenesis runner][jg-native].

HachiStep exports its embedding types at the crate root and keeps implementation
modules private. Component interfaces are available under `diagnostic`, whose stability
is separate from the embedding contract.
[Exports](../../crates/hs-core/src/lib.rs),
[machine interface](../../crates/hs-core/src/machine.rs).

Keep construction and inspection types deliberate as the API develops. Raw pin inputs
and bus traces need a clearly documented diagnostic interface. HachiStep's optional
bus observer now has a separate diagnostic type and an explicit borrowed sink. Enabling
the Cargo feature does not change the product event enum or enable observation during
ordinary execution. [Events and fixtures](../../crates/hs-core/src/signals.rs),
[bus observation](../../crates/hs-core/src/trace.rs).

Add stepping, breakpoints, or watchpoints when hachiware or a debugger needs their
behavior. Preserve stopped, side-effect-free inspection regardless of diagnostic
features. A diagnostic convenience must not create a different hardware executor.
[Debugging contract](../DESIGN.md#131-public-interface).

## Instances, threads, browsers, and mobile callers

SameBoy's linked Cocoa caller explicitly interleaves independent instances. melonDS
supplies per-instance user data to platform operations and timestamped multiplayer
packets. In contrast, libretro intentionally supports a single loaded instance, and
Boytacean's libretro adapter stores it in a global. Adapter conventions should not
force a single instance into a reusable Rust device library.
[SameBoy caller][sb-link], [melonDS platform][melon-platform],
[libretro guide][lr-guide], [Boytacean globals][bt-retro].

jgenesis's native driver owns the emulator on a runner thread and exchanges commands,
inputs, frames, and errors with the application. Its browser runs the backend through
a separate caller. This demonstrates that host threading can change while the device
API remains usable. It does not imply that the emulator itself supports concurrent
mutation. [Native thread][jg-native], [web caller][jg-web].

For HachiStep, document single-owner mutable execution and borrowed observations valid
until the next mutation. Verify `Machine: Send` and `Snapshot: Send` if callers need to
move them to a worker; callbacks should remain borrowed for the duration of a run.
Do not require a sink or error to be `Send + Sync + 'static` unless the core actually
retains it or crosses threads. Those restrictions in jgenesis follow its driver and
shared traits; they are not requirements of a synchronous sink.

Wasm and mobile wrappers need bounded calls, transferable input/output bytes, and
explicit suspension behavior. Rust's `wasm32-unknown-unknown` supports `std`, but file
operations fail and thread creation panics. `no_std` alone does not solve browser
integration. [Rust target documentation][rust-wasm]. Copying owned output across an
FFI/Wasm boundary may be appropriate even when Rust consumers borrow buffers.
Boytacean exposes both forms. [Native buffers][bt-buffers], [copied Wasm buffers][bt-wasm].

Keep host clocks, files, sleeping, UI lifecycles, and connection transport outside the
hardware core. A mobile application must choose whether suspension means pausing,
executing elapsed device time, or editing known firmware values before resuming.
The core's general stopped edits and exact snapshots support those choices without
a phone-specific progression API. [Suspension policy](../DESIGN.md#135-execution-pacing-and-external-connections).

## Implications for API freeze

The comparisons support the existing single-device, timed execution design. The
embedding contract should preserve these boundaries:

1. Keep implementation modules private and name public construction and observation
   types at the crate root. Component experiments use `diagnostic` and remain outside
   the embedding contract.
2. Use a typed run-return reason and distinguish host delivery failures from core
   faults. Replace prose-only categories where a caller needs to choose a response.
3. Group persistent construction inputs and publish their ownership/copy rules.
   Keep ordinary EEPROM saves distinct from exact running states.
4. Exercise the contract with the existing replay/CLI consumer, hachiware adapter,
   and a two-instance optical environment. Cover same-time batches, stopping at I/O,
   resumed consumption, rejected inputs, failed restore, and callback failure.
5. Establish a real browser or mobile caller before freezing FFI representations,
   buffer transfer rules, or lifecycle helpers. Until then, preserve the host-independent
   Rust boundary and measure frequent bounded calls with output enabled and discarded.

Some choices need actual consumers. A source comparison cannot decide whether a phone
frontend wants an integrated frame/audio convenience, whether a hardware bridge needs
compact waveform batches, which debugger stop conditions hachiware needs, or whether
shared appearance effects earn their code. These can be added through small helpers
after their timing, retention, and error requirements are known. Compatibility versions,
dynamic plugin discovery, a universal emulator trait, and background threads in the core
have no demonstrated need in this repository.

[pw-api]: https://github.com/h4lfheart/pocketwalker/blob/2f3b4512a668e3b7c321f213c1c8d5344627e96e/core/pokewalker/pocketwalker.h
[pw-run]: https://github.com/h4lfheart/pocketwalker/blob/2f3b4512a668e3b7c321f213c1c8d5344627e96e/core/pokewalker/pocketwalker.cpp#L32-L69
[pw-consumer]: https://github.com/h4lfheart/pocketwalker/blob/2f3b4512a668e3b7c321f213c1c8d5344627e96e/desktop/src/qt/emulator/emulator_context.cpp
[pw-network]: https://github.com/h4lfheart/pocketwalker/blob/2f3b4512a668e3b7c321f213c1c8d5344627e96e/desktop/src/qt/network/qt_network_system.cpp
[pw-sci]: https://github.com/h4lfheart/pocketwalker/blob/2f3b4512a668e3b7c321f213c1c8d5344627e96e/core/soc/sci3/sci3.cpp#L63-L123
[ps-api]: https://github.com/jpcerrone/pokestroller/blob/8a7b85df005649f61bc3cc2e8aac821fd26a787a/src/walker.h
[ps-consumer]: https://github.com/jpcerrone/pokestroller/blob/8a7b85df005649f61bc3cc2e8aac821fd26a787a/src/win_main.c#L122-L185
[ps-init]: https://github.com/jpcerrone/pokestroller/blob/8a7b85df005649f61bc3cc2e8aac821fd26a787a/src/walker.c#L2820-L2870
[powar-context]: https://github.com/UnrealPowerz/powar/blob/df3ca7ec82ea4e65703edce4fca20fd0b0b8a6da/main.h
[powar-loop]: https://github.com/UnrealPowerz/powar/blob/df3ca7ec82ea4e65703edce4fca20fd0b0b8a6da/main.c#L3225-L3301
[powar-web]: https://github.com/UnrealPowerz/powar/blob/df3ca7ec82ea4e65703edce4fca20fd0b0b8a6da/static/app.js#L17-L35
[sb-api]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/gb.h#L884-L1015
[sb-execution]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/gb.c#L1190-L1248
[sb-link]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Cocoa/Document.m#L541-L583
[sb-debugger]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/debugger.h
[mgba-api]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/include/mgba/core/core.h#L37-L185
[mgba-consumer]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/platform/sdl/main.c#L205-L280
[mgba-thread]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/core/thread.c#L250-L310
[mgba-keys]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/gba/core.c#L965-L990
[mgba-memory]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/gba/core.c#L1044-L1110
[melon-api]: https://github.com/melonDS-emu/melonDS/blob/906e9ebb27da8c6a715cd7abab4abfe8a8d29427/src/NDS.h#L327-L425
[melon-platform]: https://github.com/melonDS-emu/melonDS/blob/906e9ebb27da8c6a715cd7abab4abfe8a8d29427/src/Platform.h#L251-L315
[melon-consumer]: https://github.com/melonDS-emu/melonDS/blob/906e9ebb27da8c6a715cd7abab4abfe8a8d29427/src/frontend/qt_sdl/EmuThread.cpp#L255-L404
[jg-traits]: https://github.com/jsgroth/jgenesis/blob/cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f/common/jgenesis-common/src/frontend.rs#L184-L395
[jg-errors]: https://github.com/jsgroth/jgenesis/blob/cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f/backend/gb-core/src/api.rs#L28-L47
[jg-gb]: https://github.com/jsgroth/jgenesis/blob/cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f/backend/gb-core/src/api.rs#L126-L322
[jg-native]: https://github.com/jsgroth/jgenesis/blob/cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f/frontend/jgenesis-native-driver/src/mainloop/runner.rs
[jg-web]: https://github.com/jsgroth/jgenesis/blob/cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f/frontend/jgenesis-web/src/lib.rs#L254-L282
[bt-api]: https://github.com/joamag/boytacean/blob/2eb66337ad1732087c26076ec5bd7d08fe9f6e93/src/gb.rs#L638-L780
[bt-buffers]: https://github.com/joamag/boytacean/blob/2eb66337ad1732087c26076ec5bd7d08fe9f6e93/src/gb.rs#L1312-L1353
[bt-wasm]: https://github.com/joamag/boytacean/blob/2eb66337ad1732087c26076ec5bd7d08fe9f6e93/src/gb.rs#L884-L898
[bt-retro]: https://github.com/joamag/boytacean/blob/2eb66337ad1732087c26076ec5bd7d08fe9f6e93/frontends/libretro/src/core.rs
[lr-run]: https://github.com/libretro/libretro-common/blob/0a2b63d903ea0cc7bac9e759af57db190b4aeadd/include/libretro.h#L8562-L8614
[lr-memory]: https://github.com/libretro/libretro-common/blob/0a2b63d903ea0cc7bac9e759af57db190b4aeadd/include/libretro.h#L8696-L8724
[lr-guide]: https://docs.libretro.com/development/cores/developing-cores/
[rust-wasm]: https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html
