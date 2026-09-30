# Emulator execution and optimization

Research checked 2026-09-30. This note examines how emulator cores pursue hardware
fidelity, execution speed, and maintainable source. The examples cover handhelds,
consoles with multiple interacting processors, and standalone CPU libraries, including
Rust implementations. They include projects already referenced by HachiStep and
additional projects with different hardware and implementation choices.

HachiStep's principles remain in [DESIGN](../DESIGN.md): one coherent hardware model,
full fidelity by default, shared mechanisms for retail and custom firmware, portable
execution, exact suspension/restoration, and complexity justified by its benefit.
Projects with multiple CPU backends contribute examples of performance work outside
those backends. Their backend count is not a recommendation for HachiStep.

Representative production calls establish what the source does. Developer reports
establish stated goals and historical results. Neither establishes a universal accuracy
or performance ranking. No upstream benchmarks or hardware tests were rerun here.

One CPU implementation, one authored instruction definition, one hardware owner, and
one literal control-flow route are different properties. Identify the property a
source demonstrates before using it as a precedent. Internal event timing and the
host's permitted suspension points also need separate examination.

## Comparison map

| Project and target | Mechanisms examined | Consequence to investigate |
| --- | --- | --- |
| SameBoy, GB/GBC | Deferred display/audio work, guarded line rendering, timed access conflicts | Which observations force deferred work to become explicit? |
| mGBA, GB and GBA | Shared staged GB execution, event deadlines, active memory regions, range rendering | How much scheduling and routing work can disappear between interactions? |
| NanoBoyAdvance, GBA | Bus-driven timing, retained CPU/DMA overlap, lazy timers, delayed writes | Can concurrent hardware work have a compact serialized host representation? |
| Mesen CE, NES | Read-time reconstruction of PPU state, event-sensitive audio, compile-time instrumentation | Which internal facts need storage, and which can be derived when observed? |
| melonDS, DS/DSi | Physical-bank invalidation and coherent linear VRAM views | Can a better representation eliminate repeated mapping work? |
| jgenesis, Rust multi-console cores | Contiguous audio history, bounded DMA advancement, clock relationships | What do data layout and other active components permit? |
| ares, SNES example | Interaction-driven synchronization and readable bus operations | Where do source clarity, synchronization cost, and state capture conflict? |
| Dolphin, GameCube/Wii | Shader compilation latency and recorded GPU-command replay | Does specialization improve throughput while worsening first-use latency? |
| GameRoy, Rust GB | Deferred devices, interrupt prediction, detailed reference checks | Which prediction and advancement claims survive source inspection? |
| floooh/chips, 6502/C64 | Generated flat cycle state machine, pin interface | How much continuation machinery is necessary for fine suspension? |
| MAME, H8 | One instruction description generating entry and resume code | Can authored semantics stay coherent while removing repeated dispatch? |
| rs80, Rust 8080 | Static opcode specialization, register layout, flags, bounded memory | Which compiler-visible invariants matter independently of a JIT? |

## SameBoy

Inspected revision
[`213a12ce93d66b105a113debd9396306066a7cfc`][sb-revision]. The observations below concern
the Game Boy/Game Boy Color engine and representative default display/audio routes.
They do not describe every Super Game Boy mode or frontend option.

### CPU execution and timing

Observed call route:

```text
GB_run
  GB_cpu_run
    cycle_read for opcode fetch
    opcodes[opcode] instruction handler
      cycle_read / cycle_write / cycle_no_access
        GB_advance_cycles
          timers, DMA, display, APU, joypad, infrared, RTC
    flush_pending_cycles
```

`GB_run` invokes one opcode interpreter. `GB_run_frame` repeatedly invokes `GB_run`
until vertical blank; it does not select another CPU engine. Ordinary instruction
handlers directly compute register/flag results and call shared timed-access helpers.
For example, `call_a16` fetches the destination, accounts for the stack/OAM cycle,
writes the return address high byte and low byte, then assigns the PC.
[Public runner][sb-run], [opcode dispatch][sb-cpu], [CALL][sb-call].

The helpers accumulate unused internal cycles in `pending_cycles` and flush them
before accesses and before returning. A read advances the prior cycles, reads memory,
then records four pending T-cycles. Writes choose a conflict rule based on the register
and hardware model. Some shift a write by one or two T-cycles, and some split the
advance around a transient register condition. This preserves interaction timing
without storing a continuation record after every CPU arithmetic operation.
[Access timing][sb-access], [cycle accumulation][sb-pending].

`GB_advance_cycles` handles CPU/peripheral speed relationships, speed-switch boundaries,
and calls the peripheral owners. HALT and STOP still advance peripherals through that
route. The main interpreter samples interrupts at its defined points and models the
stack writes and later interrupt-vector selection separately.
[Clock advance][sb-clock], [interrupt execution][sb-cpu].

This is evidence for one instruction engine with timed internal interactions. It is
not evidence for externally resumable execution at an arbitrary T-cycle. `GB_cpu_run`
finishes its instruction or interrupt operation before returning. The public runner
accepts no exclusive time horizon. Saving asserts that the instance is not running
and writes retained state; it does not capture the C instruction handler's call stack.
Peripherals can remain partially advanced through their explicit states at this
boundary. [Runner][sb-run], [save entry points][sb-save].

The distinction matters when borrowing the design. Direct instruction handlers are
compact and avoid repeated continuation stores, but an embedding contract that must
suspend between any two accesses needs an additional representation of unfinished
CPU work. That is a consequence of the two contracts, not a criticism of SameBoy's.

### Display batching and separate rendering algorithms

SameBoy's PPU has an explicit state machine with retained cycle balance and state
labels. `GB_SLEEP` records the next state when supplied cycles are exhausted.
`GB_BATCHPOINT` can instead retain the current state and accumulated cycles until
enough work is available. `GB_display_sync` calls the same display owner with `force`
set, disabling further waiting for a larger batch. [State-machine helpers][sb-sm],
[forced synchronization][sb-display-header].

Mode 2 can defer the OAM scan while DMA and OAM blocking do not require immediate
processing. Mode 3 has a more substantial optimization. `mode3_batching_length` rejects
batching for relevant DMA/HDMA, STOP, window edge cases, and some model states. With
no visible objects or enabled window, it computes a duration from fractional scrolling.
For more complicated lines, it permits a 300-T-cycle interval only when the HBlank
STAT interrupt cannot affect CPU admission and HBlank HDMA is absent.
[Eligibility rules][sb-batch-guard], [OAM and Mode 3 dispatch][sb-batch].

If sufficient deferred cycles accumulate, Mode 3 calls `render_line` or
`render_line_sgb`. Otherwise it enters the FIFO/fetcher loop, using
`render_pixel_if_possible`, tile fetching, and object fetching. The fast renderer
constructs an object buffer and draws background/window tiles directly; it does not
execute every operation of the FIFO renderer. There are separate rendering algorithms
inside one PPU owner. They share some helpers and final state handling, but a literal
claim that every pixel follows one algorithm would be false.
[Line renderer][sb-line], [FIFO route][sb-slow], [shared completion][sb-join].

Forced synchronization is what makes deferral useful beyond static scenes. VRAM/OAM
accesses, relevant LCD/STAT/scroll/palette/HDMA registers, and writes to the interrupt
enable register settle display work before the access. An access before the full
batch is available therefore prevents the fast-line branch and processes the retained
elapsed work through the detailed route. At successful fast-line completion the code
updates pixel position and elapsed line cycles, sleeps for the chosen interval, and
joins common post-Mode-3 handling. [Memory synchronization][sb-memory-sync],
[VRAM writes][sb-vram], [interrupt enable][sb-ie], [dispatch and completion][sb-batch].

The 300-T-cycle case is particularly instructive. It can postpone materializing
internal mode transitions when the guarded interrupt/DMA conditions make them
unobservable; a relevant CPU access forces materialization. This is an observation of
the implementation's guards, not a proof that every possible observer is covered.
Equivalence also depends on the values retained for later glitches and fetcher state.
The common completion code and detailed route contain comments identifying timing
questions and behavior still requiring verification. [Guard][sb-batch-guard],
[completion][sb-join].

The strength of this approach is avoiding pixel-fetcher work on ordinary lines while
retaining the detailed route for interactions. Its maintenance cost includes two
rendering algorithms, complete observation guards, and agreement about post-line
state. Those costs are explicit and can be justified by substantial measured savings;
algorithm duplication alone does not settle the quality of the design.

### Audio, memory, and state

APU work is also lazy. `GB_apu_run` retains accumulated channel cycles and returns
unless output sampling, maximum sample spacing, or a sensitive channel operation
requires processing. Long intervals split at sample boundaries. Register writes force
APU advancement before changing configuration. This optimization combines hardware
conditions and output needs, rather than requiring each CPU cycle to generate audio.
[APU advancement][sb-audio], [APU writes][sb-audio-write].

Memory uses a fixed address-page dispatch table. General reads/writes still handle
DMA restrictions, data-bus behavior, optional watchpoints, and callbacks before calling
the selected owner. This is a small routing optimization beneath the timed bus contract;
it is not an immutable-code assumption. [Read routing][sb-read], [write routing][sb-write].

Native state serialization writes separate CPU/core, DMA, timing, audio, RTC, video,
and other sections. The timing section retains the display's cycle balance and state;
the video section retains its other hardware fields. Saving does not require every
component to finish its interval first. This can preserve lazy
representations at the legal API boundary. It does not establish cross-algorithm or
cross-emulator equivalence by itself. [State traversal][sb-state], [state fields][sb-fields].

### Evidence and limits

SameBoy's README declares high accuracy, T-cycle LCD timing, and named test-suite
coverage. Its contribution guidance asks contributors to verify emulation differences
against matching hardware/model revisions. The checked CI sanity script runs selected
audio/OAM/display test ROMs and compares resulting image hashes.
[README][sb-readme], [contribution guidance][sb-contributing], [CI tests][sb-tests].

The 0.15 release notes report up to 34% improvement from PPU fast paths with unchanged
accuracy, and up to 17% from lazy APU output. These are the developer's historical
results. They support the intended benefit and published experience; they are not a
fresh benchmark of the inspected revision or a formal equivalence argument.
[Release notes][sb-changelog].

No benchmark or test execution was performed for this research. Inspection confirms
that guards, deferred state, forced synchronization, and alternate render algorithms
exist. The README, release notes, and CI design establish stated goals and validation
practice. They do not establish universal accuracy, the completeness of the guards,
or performance superiority on every workload.

## mGBA Game Boy core

Inspected revision
[`c3c8e5e813f245028de118a56734e1dc0f35ce2a`][mg-revision]. mGBA's Game Boy engine uses
the SM83 implementation under `src/sm83`. Its Game Boy Advance engine uses the ARM
implementation under `src/arm`. These have different execution organizations and must
not be described interchangeably.

### Shared CPU operations with guarded timing shortcuts

Observed call route:

```text
mCore runFrame / runLoop / step
  SM83Run / SM83Tick
    _SM83TickInternal
      _SM83Step for fetch or memory operation
      idle T-state advancement and due-event processing
      current instruction-stage handler
    GBProcessEvents
      mTimingTick
        due peripheral callbacks
```

The core's frame and loop entries call `SM83Run`. Its instruction step repeatedly
calls `SM83Tick` until the CPU reaches `SM83_CORE_FETCH`. Both use the same
`_SM83TickInternal`, `_SM83Step`, and instruction table. Multicycle instructions
retain their next stage as a handler and execution state. For example, CALL stages
fetch the address, write the return PC, and select the next memory/store operation.
[Core entries][mg-core], [SM83 executor][mg-sm83], [instruction stages][mg-isa].

The executor does not always check events after every T-state. If the next event is
far enough away, it advances the two idle T-states together. Near the cached event
deadline, it increments execution state and checks/processes events between those
T-states. The fetch/memory operation and instruction-stage handler are shared by both
branches. This is a narrow guarded shortcut around scheduling work, not a separate
instruction-semantics table. [Internal tick][mg-sm83].

The source retains a limitation at the host exit boundary. `SM83Run` finishes until
its state is FETCH after event processing asks it to stop. `GBProcessEvents` explicitly
comments that its blocked-device loop cannot exit early until mid-M-cycle exits are
handled. `_GBCoreSaveState` advances the CPU to FETCH before serializing. The frame
callback similarly reschedules itself when the CPU is not at FETCH.
[CPU exit][mg-sm83], [event processing][mg-events], [state capture][mg-core],
[frame callback][mg-video].

Fine-grained internal event handling and arbitrary external suspension are therefore
different capabilities here. Completing to a legal CPU boundary simplifies capture
and callbacks. A contract requiring capture at the unchanged arbitrary instant has
more obligations. Neither choice determines the whole emulator's accuracy.

### Scheduler, timers, and halted execution

`mTiming` maintains an ordered event list and updates a cached next-event time on
scheduling. Same-time events use a priority. Due callbacks receive `cyclesLate`, and
recurring owners subtract it when scheduling their next deadline. This avoids drifting
a recurring peripheral because its callback was serviced after its nominal time.
[Scheduler][mg-timing], [video deadlines][mg-video], [timer deadlines][mg-timer].

The Game Boy timer batches divider increments to the next visible DIV change or TIMA
edge. Its shared increment routine processes the pending increments and handles timer
overflow and audio-frame timing. Resetting DIV or changing TAC first accounts for the
elapsed fraction and materializes the pending old state, then applies the change and
reschedules. TIMA/TMA writes inspect the pending reload interval explicitly.
[Timer][mg-timer], [timer register writes][mg-io].

When halted or blocked, event processing can advance directly to the next event and
update the CPU's subcycle alignment instead of executing empty instructions. These
optimizations follow interaction deadlines and retained phase, rather than a second
inactive-device model. [Event processing][mg-events].

These mechanisms remove repeated checks and unobservable increments. Their correctness
depends on the cached deadline, every owner notifying the scheduler, and accurate
rules for accesses during pending work. The source shows the mechanism; an independent
hardware test is still needed to verify each rule.

### Memory and display work

The memory owner installs a specialized cartridge-fetch function and an active-region
pointer/mask. Sequential instruction fetches use the selected bank directly until they
cross its boundary. Other regions and special mapper reads use general memory access.
Mapper writes reselect the active region, and DMA restrictions can substitute blocked
storage. This reduces address routing while keeping the supplied bytes and
device-specific access owner. [Fetch routing][mg-memory], [mapper writes][mg-memory-write].

The GB PPU uses scheduled mode transitions and a range renderer. At Mode 2 completion
it selects visible objects and computes Mode 3 duration from a base, object count, and
scroll offset. `GBVideoProcessDots` converts elapsed time into an X coordinate and
draws the newly visible range. Relevant register writes settle that range before
changing renderer settings. The software renderer processes background/window/object
ranges and includes unrolled eight-pixel output groups.
[Video timing][mg-video], [video register writes][mg-io], [range renderer][mg-render].

This route is not SameBoy's FIFO route with an optional full-line substitute. It is
a different representation of display timing and output. Its range approach keeps
ordinary rendering compact and permits mid-line register effects without simulating
every fetch phase. The inspected mode calculations and TODO comments also expose
where it does not document a complete model of individual fetch/interrupt phases.
Those are declared uncertainties in the source, not newly reproduced failures.
[Mode calculation and comments][mg-video].

The appropriate comparison is consequently broader than CPU dispatch. The visible
hardware contract depends on scheduler timing, memory restrictions, rendering state,
and register-conflict handling together. A finely staged CPU does not automatically
make all peripheral interactions cycle-exact.

### Goals, GBA contrast, and evidence limits

mGBA's README states a combined speed/accuracy goal primarily for GBA emulation and
lists GB/GBC support separately. The developer's 2017 article explicitly distinguishes
the then cycle-count-oriented GBA engine from the cycle-accuracy-oriented Game Boy
engine, describing batching and splitting around interactions for the latter.
[README][mg-readme], [developer explanation][mg-article].

The current ARM route still dispatches an ARM/Thumb instruction as a whole and checks
events around instructions, while the current SM83 route uses retained M-cycle stages
and selective T-state checks. Atomic instruction execution reduces continuation and
control overhead, and can be an effective fit for a system's supported interactions.
It also has different limits for events that must intervene during an instruction.
The source distinction supports discussing the tradeoff; a historical article alone
cannot establish the current accuracy of either engine. [ARM route][mg-arm],
[SM83 route][mg-sm83].

The 2018 developer article describes interrupt-phase research, SameBoy's then ability
to run Pinball Fantasies, and mGBA's then video-timing limitations. That is historical
research context, not evidence that the current revision still fails that game.
[Historical timing account][mg-history].

The inspected GB local tests cover construction/reset, image recognition, mapper
behavior, RTC behavior, and memory patching. Those are useful regression checks, but
their existence does not establish coverage of every CPU/PPU interaction discussed
above. GBA hardware-suite results also cannot be transferred to the GB engine merely
because both ship in mGBA. [GB test selection][mg-tests], [memory tests][mg-memory-tests].

This research does not supply fresh hardware tests, exhaustive test-suite results, or
paired performance samples. The current source confirms shared CPU operations,
guarded scheduling shortcuts, event-based peripherals, specialized fetch routing,
and range rendering. It also confirms narrower external stop/capture boundaries than
HachiStep requires. No absolute speed or accuracy ranking follows from those facts.

## NanoBoyAdvance: represent overlap at the bus

Inspected the current Codeberg revision
[`2e95a74226a5dd345c3384f9a62d08e9593fdb1a`](https://codeberg.org/nba-emu/NanoBoyAdvance/src/commit/2e95a74226a5dd345c3384f9a62d08e9593fdb1a).
This is its Update 2 development branch. The former GitHub repository redirects
development there. Its README identifies
CPU, DMA, timers, PPU and Game Pak prefetch as cycle-accurate targets and reports AGS,
mGBA-suite and CPU-suite coverage. Those are project reports, not results reproduced
here. Its optional MusicPlayer2000 enhancement interprets a particular software sound
engine; that enhancement has a different scope from the hardware mechanisms examined
below. [README](https://codeberg.org/nba-emu/NanoBoyAdvance/src/commit/2e95a74226a5dd345c3384f9a62d08e9593fdb1a/README.md).

The ARM/Thumb interpreter fetches through the bus and dispatches an instruction handler.
Bus accesses advance the scheduler, apply width-dependent timing and handle DMA
arbitration. Ordinary RAM access remains direct after those obligations. This connects
timing to the physical interaction without a general device tick for each internal
counter increment.
[CPU](https://codeberg.org/nba-emu/NanoBoyAdvance/src/commit/2e95a74226a5dd345c3384f9a62d08e9593fdb1a/Sources/NanoBoyAdvance/Includes/NanoBoyAdvance/HW/ARM/ARM7TDMI.hh#L71),
[bus](https://codeberg.org/nba-emu/NanoBoyAdvance/src/commit/2e95a74226a5dd345c3384f9a62d08e9593fdb1a/Sources/NanoBoyAdvance/Sources/Bus/Bus.cc#L82).

CPU internal cycles can overlap DMA on hardware. `Bus::Idle` runs DMA and retains its
duration as `parallel_internal_cpu_cycle_limit`. Subsequent internal CPU cycles consume
that credit. A bus access clears the credit and imposes arbitration. The host executes
the work serially while the representation accounts for overlap. The source explains
the intended equivalence; its comment is not an independent hardware proof.
[Overlap and prefetch timing](https://codeberg.org/nba-emu/NanoBoyAdvance/src/commit/2e95a74226a5dd345c3384f9a62d08e9593fdb1a/Sources/NanoBoyAdvance/Sources/Bus/Timing.cc#L9).

Timers retain a counter and start timestamp. Reads derive elapsed prescaled increments;
overflow remains a scheduled event because it can affect interrupts, cascaded timers
and audio. Reload and control writes enter pending state and take effect one cycle
later, with separate priorities. Lazy arithmetic therefore coexists with explicit
short-lived transitions.
[Timer implementation](https://codeberg.org/nba-emu/NanoBoyAdvance/src/commit/2e95a74226a5dd345c3384f9a62d08e9593fdb1a/Sources/NanoBoyAdvance/Sources/HW/Timer/Timer.cc#L122).

This is a useful representation example for concurrent components. Applying the idea
to HachiStep would require identifying all interactions that bound the credit and
preserving it across external horizons, inputs and snapshots. The instruction-level
`Run` entry does not itself demonstrate that host contract.

## Mesen CE: reconstruct intermediate state when observed

Inspected community-maintained Mesen CE revision
[`a60e79feb4d6dcced5922d636f9211837d01e381`](https://github.com/nesdev-org/MesenCE/tree/a60e79feb4d6dcced5922d636f9211837d01e381).
The original Mesen2 repository now points to this continuation. The NES instruction
interpreter calls shared bus-cycle helpers that advance the PPU and sample interrupt
lines at separate phases. Its ordinary instruction entry completes a handler; this
example establishes internal sequencing rather than arbitrary host suspension.
[Instruction entry and bus phases](https://github.com/nesdev-org/MesenCE/blob/a60e79feb4d6dcced5922d636f9211837d01e381/Core/NES/NesCpu.cpp#L169).

A less obvious optimization appears in the OAM data register read. During sprite
fetching, the PPU derives `_oamCopybuffer` from the current phase and secondary OAM only
when software reads it. The comment explicitly contrasts that with an eight-stage
update of sprite-loading work. This removes repeated maintenance of an intermediate
fact while retaining the value at an observation point.
[PPU register read](https://github.com/nesdev-org/MesenCE/blob/a60e79feb4d6dcced5922d636f9211837d01e381/Core/NES/NesPpu.cpp#L343).

The audio owner catches up before register accesses, at interrupt consequences and
at output-buffer completion. DMC activity forces finer work because DMA and CPU stalls
are observable. Channel output uses timestamped deltas. A single audio owner changes
its work granularity according to actual interactions.
[Audio advancement](https://github.com/nesdev-org/MesenCE/blob/a60e79feb4d6dcced5922d636f9211837d01e381/Core/NES/APU/NesApu.cpp#L153).

The PPU also uses a template for its concrete implementation. The ordinary PPU supplies
inline empty instrumentation methods and shares the main scanline implementation.
This is an example of keeping optional observation costs out of ordinary processing
without independently authoring the hardware sequence.
[Default PPU](https://github.com/nesdev-org/MesenCE/blob/a60e79feb4d6dcced5922d636f9211837d01e381/Core/NES/DefaultNesPpu.h).

The source comments identify individual timing tests, assumptions and approximations.
They provide useful rationale, not fresh measurements or a completeness proof. Read-time
reconstruction requires every observer and later state-dependent operation to derive
the same value, including inspection and restoration.

## melonDS: improve the memory view used by rendering

Inspected revision
[`906e9ebb27da8c6a715cd7abab4abfe8a8d29427`](https://github.com/melonDS-emu/melonDS/tree/906e9ebb27da8c6a715cd7abab4abfe8a8d29427).
The README declares a combined correctness/performance goal. The following mechanism
belongs to GPU memory handling and the software renderer, independently of the selected
CPU backend.
[Project goal](https://github.com/melonDS-emu/melonDS/blob/906e9ebb27da8c6a715cd7abab4abfe8a8d29427/README.md).

The DS can map VRAM banks into different graphics regions. melonDS tracks modifications
in physical banks, then derives dirty logical regions from both mappings and bank
changes. Unmapping a bank to modify it and mapping it back therefore need not invalidate
all its unchanged contents. Before drawing, the software renderer refreshes coherent
linear memory views. Repeated pixel/tile reads can use those views instead of repeatedly
resolving the bank mapping.
[Invalidation rationale and implementation](https://github.com/melonDS-emu/melonDS/blob/906e9ebb27da8c6a715cd7abab4abfe8a8d29427/src/GPU.cpp#L54),
[software renderer use](https://github.com/melonDS-emu/melonDS/blob/906e9ebb27da8c6a715cd7abab4abfe8a8d29427/src/GPU2D_Soft.cpp#L141).

This illustrates a change to data representation, rather than a faster version of each
mapped read. Its costs include duplicate derived storage, dirty metadata and coherence
rules. HachiStep's LCD and bus are much smaller and less configurable. A similar cache
would need sufficient repeated conversion/routing cost to earn those costs. Hardware
state remains authoritative; removing derived views must preserve subsequent behavior.
The inspected code establishes the mechanism, not a measured HachiStep benefit.

## jgenesis: contiguous history and bounded inactive work

Inspected Rust revision
[`cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f`](https://github.com/jsgroth/jgenesis/tree/cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f).
Its architecture uses instruction-based and cycle-based CPU implementations for
different processors, with reusable console components and separate native/browser
frontends. [Architecture](https://github.com/jsgroth/jgenesis/blob/cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f/ARCHITECTURE.md).

Audio filtering retains sample history in larger contiguous arrays. A moving start
index avoids ring-wrap handling in the inner filter loop. When space runs out, a copy
restores the retained history to the beginning or end. This trades extra storage and
occasional copying for simple contiguous processing. Current source includes
target-specific vectorization too; the storage organization is useful independently of
those instructions and their portability costs.
[FIR history](https://github.com/jsgroth/jgenesis/blob/cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f/common/jgenesis-common/src/audio/fir_resampler.rs#L9),
[sinc history](https://github.com/jsgroth/jgenesis/blob/cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f/common/dsp/src/sinc.rs#L46).

Genesis execution can advance in larger intervals while a long VDP DMA holds the CPU.
The interval remains short if the Z80 is active; the source records that a larger
interval caused audio/video desynchronization in Overdrive 2. When both processors are
blocked, it advances toward the scanline boundary subject to the VDP's maximum interval.
One inactive processor alone does not establish that the whole machine can skip work.
[DMA advancement](https://github.com/jsgroth/jgenesis/blob/cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f/backend/genesis-core/src/bus.rs#L709),
[interval limit and timing uncertainties](https://github.com/jsgroth/jgenesis/blob/cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f/backend/genesis-core/src/timing.rs#L35).

The changelog also records severe intermittent audio slowdown from subnormal floating
point arithmetic. That historical failure is a reason to include silence and decaying
signals in workload selection. It does not identify a current HachiStep defect.
[Changelog](https://github.com/jsgroth/jgenesis/blob/cc10b2bdd32deb51f1f7a15efa18bae2ba20a41f/CHANGELOG.md).

## ares: synchronization costs and source clarity

Inspected revision
[`4cb8d92b441557cb6bcaf133c4cbc7f6819b1122`](https://github.com/ares-emulator/ares/tree/4cb8d92b441557cb6bcaf133c4cbc7f6819b1122),
using its SNES components. The README explicitly favors clear source over some speed
and describes coroutine/context-switch costs. This is a preservation priority with
consequences, rather than a general ranking of implementation quality.
[README](https://github.com/ares-emulator/ares/blob/4cb8d92b441557cb6bcaf133c4cbc7f6819b1122/README.md).

CPU bus operations express reads and writes at specific phases with direct code. APU
port accesses synchronize the audio processor before observing its state. Threads carry
clock positions and switch when a connected owner must catch up. This keeps hardware
sequences readable while avoiding mandatory switching to every processor at every
clock.
[Bus phases](https://github.com/ares-emulator/ares/blob/4cb8d92b441557cb6bcaf133c4cbc7f6819b1122/ares/sfc/cpu/memory.cpp),
[APU ports](https://github.com/ares-emulator/ares/blob/4cb8d92b441557cb6bcaf133c4cbc7f6819b1122/ares/sfc/cpu/io.cpp#L5),
[thread synchronization](https://github.com/ares-emulator/ares/blob/4cb8d92b441557cb6bcaf133c4cbc7f6819b1122/ares/ares/scheduler/thread.cpp#L82).

Synchronization is still substantial work. The CPU timing loop polls fine-grained
conditions and synchronizes selected components; a performance profile changes some
synchronization. State capture can synchronize to safe entry points or serialize native
coroutine stacks. These alternatives have different timing and portability obligations.
The useful lesson for HachiStep is to distinguish readable hardware sequencing from the
mechanism that suspends it, and measure synchronization itself.
[CPU timing](https://github.com/ares-emulator/ares/blob/4cb8d92b441557cb6bcaf133c4cbc7f6819b1122/ares/sfc/cpu/timing.cpp#L14),
[capture synchronization](https://github.com/ares-emulator/ares/blob/4cb8d92b441557cb6bcaf133c4cbc7f6819b1122/ares/ares/scheduler/scheduler.cpp#L71),
[thread capture](https://github.com/ares-emulator/ares/blob/4cb8d92b441557cb6bcaf133c4cbc7f6819b1122/ares/ares/scheduler/thread.cpp#L105).

## Dolphin: first-use latency and replayable component workloads

Dolphin's 2017 ubershader account documents a performance problem outside CPU execution.
Specialized shaders were fast after compilation but could cause long first-use pauses.
A general rendering-pipeline interpreter inside shaders removed dependence on immediate
specialized compilation. The hybrid implementation could compile specializations in the
background while rendering through the general implementation. Dropping draws while
compilation finished had broken effects whose results later hardware work depended on.
The article also records GPU and driver costs. These are historical project findings,
not present-day performance recommendations.
[Ubershader investigation](https://dolphin-emu.org/blog/2017/07/30/ubershaders/).

Current revision
[`5102a0339c2177575378107b76541e47cc52122d`](https://github.com/dolphin-emu/dolphin/tree/5102a0339c2177575378107b76541e47cc52122d)
retains separate pending compilation, specialized pipeline and general pipeline cache
operations. HachiStep has no comparable GPU workload. The transferable question is
whether a specialization helps sustained execution but makes construction, first use,
or short calls expensive.
[Shader cache](https://github.com/dolphin-emu/dolphin/blob/5102a0339c2177575378107b76541e47cc52122d/Source/Core/VideoCommon/ShaderCache.cpp#L139).

FIFO Player records GPU commands and permits repeated playback and inspection without
running the entire game. Its documentation preserves skipped frames' rendering work
when later frames depend on their resources. It also documents replay limitations.
This is a precedent for reproducible component workloads and failure localization,
with independent hardware tests supplying a separate accuracy basis.
[FIFO Player](https://github.com/dolphin-emu/dolphin/wiki/FIFO-Player-Overview),
[hardware-test organization](emulator-state-and-testing.md#hardware-test-suites).

## GameRoy: verify predictions and inspect complexity claims

Inspected Rust revision
[`a5acdc921c0561ed93a077622b598df0e068583c`](https://github.com/Rodrigodd/gameroy/tree/a5acdc921c0561ed93a077622b598df0e068583c).
Its CPU implementations share the device owners. The following methods apply
independently of its optional JIT: memory accesses settle affected devices, interrupt
prediction bounds deferred processing, and display work can aggregate eligible lines.
[Device state and access](https://github.com/Rodrigodd/gameroy/blob/a5acdc921c0561ed93a077622b598df0e068583c/core/src/gameboy.rs),
[PPU](https://github.com/Rodrigodd/gameroy/blob/a5acdc921c0561ed93a077622b598df0e068583c/core/src/gameboy/ppu.rs).

The timer's advancement comment says `O(1)`, but the inspected implementation still
loops over elapsed TIMA periods. It removes individual cycle work without providing
constant-time advancement for every interval. Its reference implementation advances
cycle by cycle. Randomized tests compare complete timer state and interrupt results;
additional tests check predicted interrupt boundaries.
[Advancement](https://github.com/Rodrigodd/gameroy/blob/a5acdc921c0561ed93a077622b598df0e068583c/core/src/gameboy/timer.rs#L98),
[reference checks](https://github.com/Rodrigodd/gameroy/blob/a5acdc921c0561ed93a077622b598df0e068583c/core/src/gameboy/timer.rs#L394).

Whole-device tests compare prediction-enabled and prediction-disabled runs. CPU backend
comparison also checks registers, clocks, VBlank images and optional access traces.
Those are implementation-consistency checks with concrete observers. Independent
hardware expectations are still needed to verify shared rules.
[Prediction checks](https://github.com/Rodrigodd/gameroy/blob/a5acdc921c0561ed93a077622b598df0e068583c/core/tests/check_interrupt_prediction.rs),
[observer comparison](https://github.com/Rodrigodd/gameroy/blob/a5acdc921c0561ed93a077622b598df0e068583c/jit/tests/check_jit_compilation.rs#L268).

The author's 2023 account reports a CPU speedup much larger than the whole-device gain
and identifies PPU work as the dominant remaining cost. That is a useful example of
measuring where execution time moves after an optimization. Its machine, workload and
historical comparison limitations prevent treating those figures as a current ranking.
[Developer account](https://rodrigodd.github.io/2023/09/02/gameroy-jit.html).

## Compact CPU implementations: generation and compiler-visible invariants

These examples answer the CPU organization question, while the preceding device designs
show why CPU dispatch alone cannot settle whole-machine performance.

### floooh/chips

Revision `9e88298ce56319953ac7a43213a1120359f7a3a6` supplies a generated 6502 cycle
state machine. `m6502_tick` advances one clock using a flat opcode/subcycle state and
pin mask. The C64's normal and debug loops both call the same `_c64_tick`. Explicit
continuation is present without an allocated operation object for each cycle.
The source also records a reset behavior difference from the physical 6502, so the
organization alone does not establish complete fidelity.
[CPU tick](https://github.com/floooh/chips/blob/9e88298ce56319953ac7a43213a1120359f7a3a6/chips/m6502.h#L716),
[C64 execution](https://github.com/floooh/chips/blob/9e88298ce56319953ac7a43213a1120359f7a3a6/systems/c64.h#L951),
[generator](https://github.com/floooh/chips/blob/9e88298ce56319953ac7a43213a1120359f7a3a6/codegen/m6502_gen.py).

### MAME H8

Revision `8b80cfd15d79ff2a9e60681ecaedb1f6222f59e1` authors each instruction sequence
in `h8.lst`. The generator emits straight-line entry code and a continuation switch
from that same sequence. The executor uses the resumed route for unfinished work and
the direct route for new work, with checks at memory/event boundaries. This is one
authored semantic definition with two generated control-flow forms. It demonstrates
source coherence and resumability, rather than a literal single compiled route.
[Definitions](https://github.com/mamedev/mame/blob/8b80cfd15d79ff2a9e60681ecaedb1f6222f59e1/src/devices/cpu/h8/h8.lst),
[generation](https://github.com/mamedev/mame/blob/8b80cfd15d79ff2a9e60681ecaedb1f6222f59e1/src/devices/cpu/h8/h8make.py#L65),
[executor](https://github.com/mamedev/mame/blob/8b80cfd15d79ff2a9e60681ecaedb1f6222f59e1/src/devices/cpu/h8/h8.cpp#L339).

Generated source, executable footprint and generator complexity all remain costs. The
CPU source contains known bus-timing limitations for H8S variants; MAME is an execution
organization reference here, not a timing oracle for HachiStep's target.
[Timing limitation](https://github.com/mamedev/mame/blob/8b80cfd15d79ff2a9e60681ecaedb1f6222f59e1/src/devices/cpu/h8/h8.cpp#L487).

### rs80

Rust revision `dbca63e82455267b1b8c24e24311a3db43141fab` generates a function for
each 8080 opcode. Operand selection becomes a compile-time constant. State layout,
lazy auxiliary-carry representation and fixed 64 KiB memory reduce work without
dynamic translation. The README reports that a theoretically attractive dispatch
change did not improve its measured result. This supports examining generated machine
code and measuring alternatives instead of selecting dispatch by reputation.
[Implementation account](https://github.com/cbiffle/rs80/blob/dbca63e82455267b1b8c24e24311a3db43141fab/README.mkdn),
[state and memory](https://github.com/cbiffle/rs80/blob/dbca63e82455267b1b8c24e24311a3db43141fab/src/emu.rs).

Its published throughput concerns a CPU-only flat-memory benchmark. It does not price
MMIO, concurrent devices, exact external horizons or physical clock phases. HachiStep
would need to retain those obligations and materialize any lazy flags at every observer.

## Questions these examples help resolve

The examples establish several useful techniques without selecting a new policy:

- Preserve timed access order while doing ordinary register arithmetic directly.
- Avoid repeated event checks between interactions through a guarded cached deadline.
- Retain elapsed work and force its owner to materialize it before observation/change.
- Derive intermediate hardware values from retained phase when maintaining them would
  repeat work and every observer can reconstruct them.
- Represent overlap as retained timing credit where interaction boundaries permit it.
- Advance inactive devices to consequential events while retaining their phase.
- Specialize address routing beneath the ordinary hardware-access contract.
- Track changes at the physical owner before deriving invalidation for cached views.
- Arrange retained histories for contiguous processing, with measured storage/copy costs.
- Avoid output work through lazy sampling or a separate bulk renderer when its guards
  and reconciliation earn their maintenance cost.
- Measure construction, first use and short-call latency alongside sustained throughput.

There is direct precedent for the joint accuracy/performance goal. mGBA's developer
describes starting with the aim of being both more accurate and faster than
VisualBoyAdvance, and explains interaction-bounded batching in its Game Boy engine.
SameBoy, Mesen CE and NanoBoyAdvance provide concrete examples of ordinary interpreter
execution combined with the device techniques above. Their host stop/capture contracts
do not all match HachiStep's. A literal single route is most directly illustrated here
by chips' cycle tick; the other examples need the more precise distinctions stated at
the beginning. [mGBA's account][mg-article].

For any candidate in HachiStep, identify the work removed, its observation boundaries,
and retained state first. Then examine independent hardware expectations, randomized
run partitions, exact restoration, and realistic output-enabled/discarded workloads.
These are distinct checks. A passing software comparison does not itself establish
hardware fidelity, and a source technique does not establish its benefit for this
device. The agreed evidence requirements remain in [TESTING](../TESTING.md) and
[DESIGN](../DESIGN.md#195-measurement-and-acceptance).

[sb-revision]: https://github.com/LIJI32/SameBoy/tree/213a12ce93d66b105a113debd9396306066a7cfc
[sb-run]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/gb.c#L1190-L1248
[sb-cpu]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/sm83_cpu.c#L1570-L1719
[sb-call]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/sm83_cpu.c#L1266-L1276
[sb-access]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/sm83_cpu.c#L85-L318
[sb-pending]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/sm83_cpu.c#L321-L342
[sb-clock]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/timing.c#L435-L519
[sb-save]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/save_state.c#L848-L879
[sb-sm]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/timing.h#L25-L57
[sb-display-header]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/display.h#L44-L51
[sb-batch-guard]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/display.c#L1471-L1511
[sb-batch]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/display.c#L1802-L1871
[sb-line]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/display.c#L1166-L1470
[sb-slow]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/display.c#L1872-L2042
[sb-join]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/display.c#L2043-L2135
[sb-memory-sync]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/memory.c#L471-L550
[sb-vram]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/memory.c#L1011-L1020
[sb-ie]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/memory.c#L1778-L1783
[sb-audio]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/apu.c#L815-L875
[sb-audio-write]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/apu.c#L1676-L1702
[sb-read]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/memory.c#L758-L805
[sb-write]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/memory.c#L1791-L1860
[sb-state]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/save_state.c#L552-L594
[sb-fields]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/gb.h
[sb-readme]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/README.md#L19-L34
[sb-contributing]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/CONTRIBUTING.md#L5-L13
[sb-tests]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/.github/actions/sanity_tests.sh
[sb-changelog]: https://sameboy.github.io/changelog/#version-015
[mg-revision]: https://github.com/mgba-emu/mgba/tree/c3c8e5e813f245028de118a56734e1dc0f35ce2a
[mg-core]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/gb/core.c#L745-L779
[mg-sm83]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/sm83/sm83.c#L119-L199
[mg-isa]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/sm83/isa-sm83.c#L38-L133
[mg-events]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/gb/gb.c#L983-L1032
[mg-timing]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/core/timing.c#L36-L150
[mg-timer]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/gb/timer.c#L15-L138
[mg-io]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/gb/io.c#L392-L443
[mg-video]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/gb/video.c#L628-L742
[mg-memory]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/gb/memory.c#L56-L147
[mg-memory-write]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/gb/memory.c#L349-L374
[mg-render]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/gb/renderers/software.c#L608-L715
[mg-readme]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/README.md#L1-L16
[mg-article]: https://mgba.io/2017/04/30/emulation-accuracy/
[mg-arm]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/arm/arm.c#L201-L253
[mg-history]: https://mgba.io/2018/03/09/holy-grail-bugs-revisited/#the-phantom-of-pinball-fantasies
[mg-tests]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/gb/CMakeLists.txt#L34-L42
[mg-memory-tests]: https://github.com/mgba-emu/mgba/blob/c3c8e5e813f245028de118a56734e1dc0f35ce2a/src/gb/test/memory.c
