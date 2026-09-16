# Implemented architecture

This page describes code that exists, not the unimplemented end-state design.
See STATUS for the separate hardware-fidelity boundary.

## Composition and ownership

`Machine` owns one CPU, one MCU, external M95512/BMA150/NT7508 devices, conditions,
resolved board state, a pending CPU access, and cached appointments. `Mcu` owns
flash/RAM and concrete register/peripheral owners. There is no raw SFR mirror
beside those owners and no plugin device registry.

The CPU has no access to a frontend, wall clock, filesystem or `pw` symbols. It
emits a small next-action value; the fixed integration layer performs that action
on the MCU. External devices see selected board pin edges. SSU and GPIO reach the
same device parsers. A byte parser exists only after a device has accumulated
its serial bits.

Host integration is `Output::event(Event)`. The no-op sink allocates nothing.
A host-chosen `Vec<Event>` is useful for regression tests but is neither required
nor stored by the core. Rendering a screenshot does not execute another machine.

## The single CPU continuation

`cpu::Cpu::next(interrupt)` returns one action: a read, write, internal wait or
sleep transition. `Cpu::complete(value)` commits the corresponding result and
moves the same continuation forward. The variants retain the effective address,
read halves, write payload or exception/copy progress needed at the next phase.

There is no whole-instruction runner beside a partial runner, no decoded-block
execution loop, no JIT, no native firmware replacement, and no speculative clone
and rollback. Register aliases refer to the authoritative ER register values.
The incremental decoder requests extension words only when needed.

Aliased predecrement stores capture the source after the full address-register
update. RTE does not inherit the following-instruction delay belonging to LDC.
EEPMOV.W samples NMI only between complete transfer cycles; an already-issued
read remains stable until completion. Exception entry saves the next PC and the
firmware must explicitly resume the remaining copy.

A word access to a byte-wide SFR range is represented as separate completed
lanes; a native word register remains a word access. Long transfers have multiple
word effects. The first completed lane is never undone because a later phase
fails. Exception entry writes the saved PC before the CCR word. The current
nominal timing and prefetch omissions remain listed in STATUS.

## Time and appointment semantics

`Time` and `Duration` use 64.64 fixed-point seconds in `u128`. `Clock` retains
integer period, remainder, denominator, fractional remainder and edge ordinal.
Clock inverse/advancement tests cover partitioning and strict endpoints. MCU
clock taps share their source and divider edge ordinal.

`Machine::run_until(T, inputs, output)` processes effects strictly before `T`.
Effects at `T` remain pending. An input at the exclusive endpoint is not consumed.
The return value includes `inputs_consumed`, which the caller must use when
continuing a timeline. The complete supplied slice is validated before mutation.

CPU waits, SSU half-edge waits and ADC aperture/result waits retain a `ClockWait`
with remaining source edges and a revision-tagged derived deadline. Downstream
gating pauses the obligation; a source change recomputes its appointment. SCI
and stabilization still have older absolute appointments, and source switching
still rephases fractional timing; these remain explicit fidelity work.

The implementation compares the pending CPU completion, the cached next-device
appointment, the next input, any wake delay and the requested horizon. It does
not scan every peripheral after every ordinary RAM access. MCU register accesses
synchronize the relevant digital mechanisms; explicit device boundaries handle
serial edges, conversion apertures, programming completion and sensor updates.

At an equal timestamp the present kernel settles device events, then applies the
input batch, then completes a CPU effect. That ordering is reproducible, not a
universal silicon claim. Timer W now resolves its documented write/capture/
buffer and counter-clear conflicts locally, and the comparator/AEC owners retain
read qualification independently of controller flags. These do not establish a
universal precedence for every other register.

## Fixed serial wiring

GPIO/function selection resolves LCD CS, LCD command/data, external EEPROM CS,
BMA select, clock/data and the return net. Rising/falling edges reach the owners;
select changes reach them immediately. Opposing external push-pull drivers stop
with a specific diagnostic instead of arbitrarily choosing a winner.

The LCD's partial-byte state and pending-command-parameter state are separate.
Deselect can discard a partial serial byte without losing the parameter expected
by a previous completed command. EEPROM selection and programming lifetime are
also separate: programming continues after the accepted command's deselection.

Some mux/pull/electrical cases and other serial modes are unsupported. The fixed
wiring model does not infer recipient intent from a firmware PC or a command byte.

## Persistent domains

M95512 array bytes and nonvolatile status bits are distinct from its volatile
write-enable/busy/parser state. BMA150 nonvolatile bytes are distinct from its
working image. Their commits emit `NvByte` changes followed by `NvCommit` at the
modeled completion time. The CLI writes only explicit new output files after the
run. Original files are not silently synchronized.

Internal flash is an authoritative image but not yet a functional programming
owner. Reads and execution work; program/erase requests stop explicitly.

`Machine::with_persistent_state` can restore the external EEPROM/status and the
sensor's 19-byte nonvolatile image without restoring volatile session state.
A session checkpoint is a different operation.

## Snapshot and lifecycle

`Snapshot` owns a full typed copy of causal machine state. Restoring one preserves
partial instruction fetch/access, serial shifts, timer/clock state, device
histories and in-flight programming. It is an in-memory Rust type, not a portable
binary file format. Construction, snapshot creation and diagnostic projection can
allocate; the hot run loop does not.

MCU reset aborts CPU work while retaining RAM and external device lifetimes as
modeled. Whole-product power removal stops the powered machine; power restoration
creates the documented canonical cold volatile state and preserves nonvolatile
images. Power loss during programming is rejected before mutation rather than
inventing a partial-programming outcome. These operations are explicit and have
regression tests. Analog supply input does not currently generate brownout/reset.

## Failure contract

Image-size/status errors reject construction. A bad, backward or conflicting
input slice rejects the call before executing it. An unsupported hardware effect
or decoder form latches a model fault: subsequent run calls return that fault
without advancing. Effects already completed before the fault remain present.
A snapshot or a supported full power cycle can restore a runnable state.

A model error is not a hardware exception. The CLI reports time, PC, current
instruction PC, continuation, register state and the specific error. The core
never patches the firmware to keep a failed run going.

## Current extension seams

Add instruction forms in the decoder and the one CPU continuation. Add register
semantics in the owner and address routing in `mcu/mod.rs`. Add actual independent
time obligations to the affected owner's deadline calculation; update the fixed
integration when a new connected consequence exists. Do not add a callback
registry or cross-owner `RefCell` graph to avoid understanding ownership.

The public modules are exposed for development and fixture access. Their internal
representation is not declared stable. The intended embedding surface is the
`Machine`/`Images`/`Conditions`/input/output facade documented in API.

## New concrete owners and test seams

The AEC keeps OVH/OVL flags separate from its emitted controller requests. PWM
output and external gate/clock changes feed its one counter recurrence, including
the documented gate-return counting effect. Its P12 PWM route also reaches the
actual EEPROM select net; it is not merely a detached test pin.

The comparator keeps its analog output and read-armed comparison baseline
separate. CMDR inspection does not arm/acknowledge it. The P30 VCref path retains
the SCI transmit pin independently. Response timing remains a nominal inertial
witness, not an analog measurement.

`AnalogPin`, `DigitalPin` and `NmiPin` inputs are hardware-fixture seams, not new
physical buttons or claims that every pad is exposed on the retail enclosure.
Duplicate assignments are rejected before mutation. NMI is a dedicated latch
and is acknowledged when the CPU actually admits vector 7, not when a maskable
request is inspected.
