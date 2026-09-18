# HachiStep design

This document defines required behavior and architectural choices. Revise a choice when
concrete consequences justify it. Track implementation progress and completed work in Git
and the task discussion.

HachiStep models one physical Pokéwalker through one statically compiled, resumable
interpreter. Compact representations of hardware evolution reduce host work while
preserving observable effects and their timing.

Given the same initial physical state and input history, regrouping host work must
preserve every hardware effect, its timing and the state that determines future effects.
Performance is measured in the resulting default build.

## 1. Target and fidelity

### 1.1 Hardware and evidence

Model the actual Pokéwalker, including physical conditions and power transitions.
Introduce revision distinctions only when evidence establishes a hardware change. The
target includes the H8/38606, board connections, EEPROM, accelerometer, display
controller, infrared circuit, buttons, buzzer and supply behavior. The target addition
specifies 48 KiB of flash and 2 KiB of RAM.
[Board investigation][1], [Renesas manual][2].

| Question | Evidence |
| --- | --- |
| Memory and flash geometry | H8/38606 addition and applicable corrections. |
| Instruction semantics | H8/300H software manual with the MCU's restrictions. |
| MCU timing, registers, clocks and power | H8/38602R manual and target amendments. |
| External components | Applicable datasheets, board observations and measurements. |
| Connected pins | Board evidence and firmware accesses. |
| Firmware behavior | Matching `pw` decompilation and observed execution. |

`pw` is strong evidence of what the real device executed successfully. Read full
instruction sequences and their surrounding state to infer the mechanism that also
explains custom firmware. Treat comments as interpretations and examine the actual
accesses when they differ.

Before declaring behavior undocumented, search the relevant manuals, additions, errata,
component documents and firmware thoroughly. Reconcile conflicts between sources.
Strong, coherent inference is sufficient to implement behavior; record its basis beside
the mechanism and refine it when stronger evidence appears. An absent datasheet sentence
alone does not justify a halted or missing function.

### 1.2 Observable behavior

Retail and custom firmware use the same hardware mechanisms. Preserve memory and
register values, physical access widths, timing, side effects, interrupts, reset
sequences, connected signals, display history, sound, infrared and persistent
operations. Retain all hidden state that can affect later behavior. Firmware identity
and recognized routines must not select hardware shortcuts.

### 1.3 Physical parameters

Instruction effects, commands and latch rules belong in the implementation. Explicit
unit parameters describe frequencies, calibration and analog behavior. Selected values
need a stated basis; unresolved details stay local to the model. Every configuration
uses the complete hardware model.

Identical initialization and timestamped inputs must produce identical hardware
observations on native and browser hosts running the same core revision. Any modeled
noise starts from reproducible initialization and retains its state in captures. Host
clocks and call partitioning must not change hardware behavior.

## 2. Execution model

Each hardware component owns its state. Synchronization occurs when that state can
affect another component or produce an output. Between those interactions, a component
may advance with exact arithmetic or a compact signal description. The grouping must
preserve every observable consequence, including its timing.

| Mechanism | Responsibility |
| --- | --- |
| Timed CPU executor | Execute instruction effects and retain progress at suspension. |
| MCU bus | Resolve addresses, physical widths and access timing. |
| Clocks | Preserve source phase, divider relationships, gates and transitions. |
| Scheduler | Find the next interaction that bounds execution. |
| Board wiring | Propagate actual pin, interrupt and device connections. |

Use concrete components with fixed connections. One resumable implementation performs
each effect, including complete instructions, suspended instructions, RAM execution and
serial transfers. A replacement execution technique must replace that implementation
throughout production. This excludes a second interpreter, a JIT fallback, firmware
substitutions and speculative rollback.

Near's synchronization model provides a useful precedent for advancing components when
they interact. HachiStep retains that principle with explicit state instead of a
coroutine stack per component. [Synchronization design][3]. MAME's paired
normal/restarted instruction implementations explain another tradeoff; the single
executor requirement selects a different organization.
[MAME CPU design][4]. Correct cycle totals must also preserve the order of
interactions within them. [mGBA's accuracy discussion][5].

## 3. Code organization and authority

### 3.1 Modules

Use one core crate with modules for the CPU, MCU peripherals, external devices, time
arithmetic and board integration. The machine composes these modules and controls
execution. Split a module when a substantial mechanism or navigation benefits from its
own file.

### 3.2 Authority

| Behavior | Owner |
| --- | --- |
| Instruction effects and ordering | CPU timed sequence and the applicable bus contract. |
| Address routing | MCU bus. |
| Register access | Owning peripheral. |
| Package-pin selection | GPIO and function selection. |
| Connections between pins | Board composition. |
| Simultaneous hardware causes | Affected component's conflict rule. |
| Next consequence | Component state, cached by the scheduler. |
| Output rendering and storage | Host adapters. |

### 3.3 State

Keep one authority for each mutable hardware fact. Derived views, such as a framebuffer
or interrupt-eligibility mask, are rebuilt from that authority. Retain physically
distinct storage, including transmit holding and shift registers, sensor nonvolatile and
working images, EEPROM page buffers and committed cells, and programmed display settings
and scan latches.

### 3.4 Dependencies and integration

Components use time arithmetic and small signal types. Board integration invokes an
owner, receives changes to its connections or appointments, and updates the fixed set of
affected neighbors. Use disjoint borrowing and compact change masks. The machine holds
exclusive mutable access during execution.

### 3.5 Shared mechanisms

Share clock arithmetic, fixed-width operations and serial shifting where their behavior
is identical. Flag clearing, reset domains, command lifetimes and programming procedures
remain with the components whose rules define them.

### 3.6 Rust implementation policy

Keep dependencies few and justified by the total complexity they remove. Zero
dependencies is not a goal in itself. Evaluate core runtime, build-time, test, and
frontend dependencies separately; adding one requires an actual need.

Use `std` for the initial Rust core. Keep files, host clocks, threads, and platform
services outside hardware execution. Revisit `no_std` when an actual consumer requires
it; no extra target or configuration matrix is needed now. Construction and explicit
save state operations may allocate; ordinary execution must not. The source comparison
and dependency rationale are recorded in
[emulator dependencies](research/emulator-dependencies.md).

Unsafe Rust is forbidden by default. An exception requires a concrete, proportionate
justification: a meaningful measured benefit or substantially less complexity than the
viable safe alternatives. Convenience alone is insufficient. Any exception must be
narrow, explain its safety invariants, and preserve the portable single execution
mechanism. Do not weaken the existing prohibition before such a case exists.

This prohibition applies to HachiStep's own code. Calling safe APIs from the standard
library or a justified dependency does not require their internals to contain no unsafe
code.

---

## 4. Time and clocks

### 4.1 Clock state

Each physical source retains its frequency, epoch, edge ordinal and fractional phase.
Each derived clock retains its source, divider, gate and reset/hold state. Shared
prescalers use one state. Preserve the location of gates relative to those dividers,
because stopping a divider and gating its output affect phase differently. Independent
oscillators retain independent phases.

### 4.2 Timestamp representation

Use an opaque 64.64 timestamp in seconds, represented by `u128`. Its resolution is
`2^-64` seconds. This bounds numerical quantization; physical frequencies still follow
their supplied parameters.

Use the wide timestamp at component and API boundaries. The CPU's frequent execution
work uses a 64-bit budget in its local clock domain. Convert between the two at
synchronization points.

### 4.3 Preserve fractional remainder

For a source frequency \(p/q\) Hz, one period in timestamp units is:

$$
D=\frac{2^{64}q}{p}.
$$

Precompute its whole part \(w\) and remainder \(r\):

$$
2^{64}q=wp+r.
$$

Advancing \(k\) periods with retained remainder \(a\) gives:

$$
\Delta t=kw+\left\lfloor\frac{a+kr}{p}\right\rfloor,
\qquad
a'=(a+kr)\bmod p.
$$

For single-period advancement, this reduces to addition and an occasional carry. Bulk
advancement needs division only at the synchronization boundary.

The arithmetic must use checked operations. Construction validates frequency
representations; impossible arithmetic is a host API/model error, never silent
wraparound.

For an integer timestamp distance `delta`, the last edge strictly before that boundary
has offset `floor((delta * p - 1 - a) / (q * 2^64))`. Handle an empty interval before
subtracting. The same result is `floor(((delta * p - 1 - a) >> 64) / q)`, so inverse
edge counting needs only a shift and a narrow division after the checked multiplication.
Subtract before shifting to preserve the exclusive endpoint. This reduction is portable
and adds no cache or second execution path.

### 4.4 Clock obligations

Retain waits as remaining work on a named clock source. A source change, gate or
stabilization interval changes when that work can finish. Preserve completed edges and
any phase retained by the hardware. Apply this rule to CPU accesses, serial dividers and
conversions.

### 4.5 External horizons

`run_until(T)` stops before effects at `T`. The caller supplies inputs through a known
horizon and delivers new inputs before execution crosses their timestamps. This permits
deterministic replay and timed environmental interaction through the same execution
mechanism.

## 5. Scheduling

Each component supplies its earliest consequence that affects another owner or requires
an output update. Examples include interrupt assertion, conversion completion and
receiver sampling. Local increments that only affect a future read can remain implicit
until that read.

### 5.1 Appointments

Use fixed appointment slots and a cached minimum. An earlier appointment may replace the
minimum. Moving another slot later leaves it valid; cancelling or postponing the minimum
requires recomputation. Collect slots due at the same timestamp together.

### 5.2 Execution

1. Find the earliest input, peripheral or caller boundary.
2. Advance the CPU toward it. Keep register arithmetic inside the CPU loop and
   synchronize the affected component when an access needs it.
3. Recompute the boundary whenever activity changes clocks, wake eligibility or
   appointments.
4. Resolve due causes, propagate changed signals, update appointments and deliver
   outputs. Repeat until the caller's horizon.

A configuration write first settles elapsed work under the old configuration, resolves
coincident activity, applies the write and updates retained phase and future
appointments according to that register's rules.

### 5.3 Simultaneous causes

The affected owner resolves coincident accesses and hardware events. Preserve transients
that last a hardware phase, even if the register returns to its prior value afterward. A
global ordering convention alone cannot express every register conflict. Execution must
stop before consequential boundaries; crossing one and replaying from a snapshot would
conceal the timing defect.

## 6. Exact compressed advancement

### 6.1 Advancement

Implement one advancement function over an elapsed edge count, including `n=1`.
Independent recurrence models in tests can check its results.

### 6.2 Exclusive edge counting

Given next unprocessed edge \(e\), period \(P\), and exclusive endpoint \(T\):

$$
n=
\begin{cases}
0,&T\le e,\\[2pt]
1+\left\lfloor\frac{T-1-e}{P}\right\rfloor,&T>e.
\end{cases}
$$

The formula applies within an integral local clock coordinate. Conversion between
independent clocks is handled separately.

### 6.3 Counter example

For modulus \(M\), current count \(c\), and \(n\) increments:

$$
c'=(c+n)\bmod M,
\qquad
o=\left\lfloor\frac{c+n}{M}\right\rfloor.
$$

The owner applies the overflow count to its sticky flags, interrupt lines, outputs and
connected counters. Scheduling work can disappear only while all those effects remain
represented.

### 6.4 Retained history

Preserve histories used by filters, synchronizers, qualification counters and serial
parsers. A delayed read still observes the effects of intervening inputs. Execute local
transitions wherever an exact compressed update is unavailable.

### 6.5 Composition

For constant configuration and an appropriately fixed input segment, advancement must
satisfy:

$$
A(A(s,a),b)=A(s,a+b)
$$

after canonicalization of equivalent lazy representations.

This identity is a concrete test obligation for each compressed mechanism. It protects
against lost fractional phase, repeated rounding, and call-size-dependent filter or
counter behavior.

---

## 7. CPU execution

### 7.1 Resumable state

Retain architectural registers and CCR, fetched material, a continuation identifier,
operands and temporary values needed after suspension, any active physical access,
remaining clock work and interrupt-admission state.

A flat continuation selects the next timed effect. Execute consecutive effects directly
while the time budget permits; store a continuation where suspension can occur.
Generated state machines provide a precedent for this organization.
[Floooh's CPU implementation][6].

### 7.2 Instruction descriptions and generation

Organize instruction source by semantic family. Each definition describes its encoding
restrictions, operands, physical access order, arithmetic, visible effects, clock
obligations, interrupt behavior and source basis.

Use deterministic generation during compilation for repetitive decode or dispatch
structures when it removes substantial handwritten work. Keep the source descriptions,
generator and generated footprint small. Hardware test expectations have their own
evidence and must remain independent of these semantic definitions.

### 7.3 Arithmetic and registers

Decode once, perform direct host arithmetic at explicit target widths, and route
physical accesses through the MCU bus. Register aliases use masks and shifts over one
register file. Define overflow, division and shifts in target terms. Each instruction
updates only the flags its hardware contract changes.

CCR is materialized directly. A future lazy representation requires measured savings and
a complete contract for materializing it at every observable exit.

### 7.4 Specialization and caching

Specialize widths and instruction families when measurements show that the saved work
justifies the code footprint. The rs80 results demonstrate why dispatch choices need
measurement. [rs80][7].

The default compact decoder feeds the single executor. A justified decode cache may
store interpretations of fetched bits. Hardware fetches retain their timing and side
effects; actual prefetched bytes survive later memory changes. RAM execution,
extension-word changes and flash programming remain valid. Clearing all decode metadata
changes only host execution cost.

### 7.5 Compiler choice

Use interpretation compiled with the host application. A JIT would add code-cache
management and precise exits to the single execution requirement. GameRoy shows both the
performance potential and the fallback cost of that approach; its peripheral
optimizations are also relevant independently. [GameRoy][8].

## 8. CPU timing and exceptions

### 8.1 Physical accesses

Retain the completed prefix of every instruction. If reset arrives between two accesses,
the first access keeps its effects. Resolve each access at its actual phase, including
cases where a later lane has a different target or outcome.

### 8.2 Exception entry

The target's normal-mode exception sequence writes the return PC at the old stack
pointer minus two, then the CCR word at the old stack pointer minus four. Retain that
order, discarded prefetches, internal waits, vector reads and handler fetches as timed
work. [Renesas exception sequences][2].

### 8.3 Interrupt admission

Retain the distinction between peripheral condition, status latch, controller request,
enables, CPU mask, request sampling, admission and exception entry. Suspend and resume
at the same admission phase. Apply the target's distinct interruption rules for
`EEPMOV.B` and `EEPMOV.W`. [Renesas interrupt rules][2].

### 8.4 Unspecified behavior

Use the evidence and inference policy in §1.1 for reserved encodings, unassigned
addresses and unusual programming sequences. Keep each selected rule with its mechanism.
Ordinary address reduction, lane selection and access order continue to apply. An
implementation limitation is a host diagnostic; guest exceptions require an actual
hardware cause.

## 9. Memory and register access

The MCU directly classifies the fixed memory regions and registers. Ordinary RAM and
flash reads use direct array access; registers dispatch to their owners. Carry physical
width and access origin through the transaction. Native word accesses and ordered byte
lanes have different timing and side effects.

An active access retains its address, target, direction, width, qualified origin, write
data or partial read data, physical phase and remaining clock work. Include instruction
information only where a peripheral uses it.

Each register contract defines storage, accepted widths, sampled read value, read side
effects, accepted write bits, resulting operations, simultaneous-event rules and
reset/power behavior. Routing owns addresses; components own semantics.

Inspection projects state at the current instant without guest read side effects. It
must preserve flags, sensor shadows, receive acknowledgments and transfer state.

## 10. Peripheral responsibilities

Component details and their evidence live in the [hardware references](SOURCES.md).
These requirements define how the components participate in the complete machine.

### 10.1 Clocks, reset and power

Represent the actual reset and power domains. Each transition determines which execution
stops, which latches reset, which clocks and phases survive, which pins change and which
external operations continue. Propagate resulting pin changes through the board. The
firmware observes reset causes and performs its own initialization and recovery.

### 10.2 Interrupt controller

The controller owns its registers and latches; components own their status flags. Cache
eligibility only as derived state and update affected requests when sources, enables,
masks, routes or power change. Preserve the consequences of firmware read/modify/write
sequences racing with hardware flags.

### 10.3 Timer B1

Retain counter, load, selected clock, phase and request state. Exact reload arithmetic
includes the first partial traversal. Apply the specific rules for stopped loads,
running loads, mode changes and source changes.

### 10.4 Timer W

Retain counter and phase, compare/capture registers, buffers, output latches, capture
history, clear qualifications and interrupt controls. Compute relevant compare,
overflow, capture, buffer and output consequences. Preserve the transient first cycle
after reconfiguration before using a repeating signal description.

### 10.5 RTC

Retain divider phase, raw registers, update/busy state, controls and periodic
conditions. Reads project values at their actual update phase. `pw` waits for busy to
clear, takes two time snapshots and retries until they agree. Its deferred minute/hour
processing can coalesce during infrared activity; let the firmware execute that policy.

### 10.6 Watchdog and asynchronous event counter

The watchdog retains write qualifications, counter progress, reset cause and reset hold.
The AEC retains its counter, gate and pulse-width state and consumes routed package-pin
activity. Each follows its own clock and reset rules.

### 10.7 SCI and infrared

Retain holding and shift registers, frame position, clock phase, receiver samples,
errors and status-clear qualifications. Logical SCI output passes through IrDA encoding
and the transmitter; incident light passes through the receiver and SCI sampling.
Adapters supply timed signals to that chain. Partial or malformed frames, overrun,
disablement and clock changes follow the same model.

### 10.8 SSU and IIC

Apply actual function selection and shared-pin rules. Hardware serial and GPIO
bit-banging reach the same resolved nets and external devices. IIC retains open-drain
drives, START/STOP recognition, acknowledgments, arbitration and clock-stretching state.

### 10.9 ADC and comparators

The ADC retains its acquired sample, conversion progress and published result
separately. Later input changes affect later acquisitions. Comparators follow their
continuous-input response model. Supply, temperature and analog stimulus are explicit
physical conditions; firmware computes battery policy from them.

### 10.10 Internal flash and boot service

Retain cell contents, partial exposure, programming/erase controls, verification and
access restrictions. Fetches use the ordinary CPU bus. Modified cells invalidate decode
metadata while already fetched CPU bytes retain their values.

The unavailable manufacturer boot ROM is represented by a functional service using the
same bus, SCI, flash and clocks. Its private instruction timing and scratch state remain
unestablished. An authentic ROM image would allow ordinary execution to replace that
service. See the [boot contract](research/h8-boot-mode.md).

### 10.11 M95512 EEPROM

Retain serial/parser progress, write-enable/protection state, pending page/status data,
programming progress and committed cells. Apply the installed variant's capabilities and
128-byte page rules. [ST datasheet][9]. Status polling samples actual programming state.
Power interruption preserves the resulting partial cells. Firmware recovery markers and
staged updates remain individual writes.

### 10.12 BMA150

Retain the physical input response, calibration, conversion phase, filter history and
fill, published data, shadows, freshness, interrupt histories, configuration,
nonvolatile cells, sleep/wake/self-test state and serial progress. The moving average
must preserve its recurrence, rounding and startup behavior. [Bosch][10]. Any modeled
noise follows physical sample progression. Keep the basis for calibration-field
interpretations with the model.

### 10.13 NT7508

Retain serial assembly, pending parameters, RAM, addressing and plane phase, controls,
scan/latch state and drive configuration. `pw` sends some commands and parameters in
different selected intervals, so deselection ends the serial byte while a completed
command may still await its parameter. Implement the controller's addressing; firmware's
bank convention follows from its commands.

## 11. Serial advancement

Use one serial operation that consumes an exact run of edges. Bound that run by chip
selects, pin functions, external signals, clocks, transfer controls, device output
changes, parser transitions and the caller horizon. Apply shifts, samples and protocol
transitions in their hardware order. The same operation handles one edge or many.

Each device defines the lifetimes of byte assembly, command assembly, selection and
internal work. A shared shift helper supplies only the common bit operations. Resolve
active drivers, released lines and pulls on the actual board nets. Multiple selected
devices interact through those nets. Any contention rule needs an electrical basis.

## 12. Outputs and presentation

### 12.1 Signal descriptions

A stable waveform can be represented by its clock, phase, initial level, bounded
transition pattern, repetition period and start time. A configuration change ends that
segment at its actual time and begins the next with the resulting phase. The description
must reconstruct every transition. Lazy audio generation in SameBoy provides a relevant
example of reducing output work. [SameBoy][11].

### 12.2 Buzzer output

Expose the buzzer's drive law for an audio adapter to evaluate at its requested sample
rate. Resolve simultaneous changes to both terminals together so host update order
cannot introduce a pulse in their differential voltage.

### 12.3 Feedback

Retain every interrupt, capture or other hardware effect of an output edge. These
effects occur regardless of host playback settings.

### 12.4 Committed output

Consumers may expand a signal rule only through the time the core has finished
executing. Publish timestamped changes and that completed horizon. Later firmware can
replace the rule for subsequent time.

### 12.5 Display output

The LCD owner resolves command, RAM, scan and latch behavior and exposes the resulting
display state. Perform pixel conversion when requested. A frontend consumes that state
through the display interface.

### 12.6 Delivery

Use a synchronous borrowed sink. It may copy, consume or discard an output, but must not
re-enter the machine. Ordinary execution allocates nothing. Host consumers own any
retained event history.

The sink may request a return from the current run call. Finish all effects at the
current timestamp, including the input batch and further output, before returning an
exclusive horizon one representable time quantum later. Resume through the same
executor. This request is host control and does not enter captured hardware state.

Persistence events include the actual changed bytes or status and operation boundaries,
so the caller can process several changes before its next inspection.

### 12.7 Frontend integration

When building the first frontend, decide which display conversion, audio sampling,
filtering and presentation history merit helpers shared across frontends. Choose
appearance and sound models from those requirements. Hardware controller behavior and
any feedback into the machine remain core responsibilities. Presentation helpers may
convert compact outputs to pixels or samples on demand.

[Established emulator examples](research/emulator-presentation.md) inform this
future decision. Detailed panel and piezo models require a concrete use and evidence
before implementation.

## 13. Public API and save states

### 13.1 Public interface

Build the Rust core and embedding API first. Add C and JavaScript/Wasm adapters when
consumers require them. Before release, improve interfaces and update callers together.
Compatibility machinery requires a concrete external need.

The API constructs a machine from images and conditions, accepts timestamped physical
inputs, runs to an exclusive horizon, exposes observations and persistent contents,
applies power operations, and captures/restores state.

Timed execution, inspection without guest effects and optional traces provide the
initial debugging controls. Add stepping, breakpoints or watchpoints when a specific
task requires them, through the same executor. Raw pin fixtures serve hardware
diagnostics through a distinct advanced interface.

### 13.2 Inputs

Product inputs describe physical conditions:

```text
Button positions
Device-frame specific force
Temperature where relevant
Supply conditions
Incident infrared signal
```

No `add_steps`, `receive_packet_into_ram`, or `set_retail_clock_counter` operation
belongs in the hardware core.

Motion input includes a defined coordinate frame and interpolation rule. Specify whether
samples are held or interpolated between timestamps. Choose a representation for the
actual input fidelity required, and preserve its sampling semantics when adding
trajectory helpers.

The frontend converts its host sensor recording into that trajectory and accounts for
the recording's sampling limits.

### 13.3 Simultaneous inputs

Changes to independent properties at one timestamp form a batch.

Contradictory assignments to the same property at the same instant are rejected rather
than resolved by list order.

Input validation should be amortized. Do not rescan the full input history on every
short run call.

### 13.4 Save state contents

An EEPROM save preserves the persistent contents used by firmware. A save state captures
the running machine; its in-memory representation is a snapshot.

Capture every fact that can affect subsequent hardware behavior: CPU registers and
unfinished effects, fetched material, RAM and nonvolatile contents, physical conditions,
clock phases, power state, interrupt admission, peripheral histories, serial progress,
acquired analog values and programming operations. Retain a component's completion
target or remaining clock obligation; rebuild the calendar entry derived from it.

Keep decode caches, host callbacks and paths, rendered images, profiler totals and
derived scheduler data outside the serialized hardware contract.

### 13.5 Execution pacing and external connections

The core models one independently usable Pokéwalker. Each instance owns its hardware
state and clocks. Expose bounded execution, timestamped physical inputs, and timely
output delivery, including control at necessary I/O boundaries through the same
executor. A public operation taking two machines, peer identity, connection session, or
network transport does not belong in the hardware core.

The downstream application supplies the environment and connection layer. It owns peer
selection, transport, buffering, host clocks, waiting, and execution speed. The core
runs without host sleeps. At 2× the application requests two emulated seconds per host
second; CPU, peripheral, and serial timing relationships are unchanged. Ordinary speed
controls do not alter physical oscillator settings or select another accuracy mode.

Use timed incident and emitted infrared signals as the hardware boundary. Compact
waveform descriptions or batches may reduce overhead while preserving the signal and its
timing; one network message per pulse is not required. The application must deliver
those signals in time for its chosen connection. Execution cannot cross unknown input
and later insert it into the past.

Firmware performs discovery, handshakes, transfers, acknowledgments, retries, and
timeouts through the modeled hardware. The core supplies no protocol assistance or
special behavior to make communication succeed.

Running at 1× during connection attempts and communication is the recommended downstream
policy for live linking. A software environment may choose a different common pace; this
does not make pairing or speed restrictions core responsibilities. Reusable connection
helpers follow concrete frontend needs, as presentation helpers do in §12.7.

The source comparison is in [emulator linking](research/emulator-linking.md).

### 13.6 Native save state design

Build exact native save states with hardware semantics that could support future
agreement between emulators. Interoperability remains deferred. Keep the format
changeable until the project approaches 1.0, without versions or migrations.

Capture at a stopped API boundary with every operation at its existing phase. Specify
retained values, units, time references, completed effects and remaining work. Saving
and loading preserve the captured instant and perform no guest accesses. A compact
executor continuation may map to a documented hardware operation during capture/load.
Use shared types directly where they already express that meaning.

Borsh is the selected encoding. It supplies language-independent byte rules; the [native
state contract](SAVE_STATES.md) supplies the field meanings. Use explicit integer widths
and enum tags, shared encoder/decoder definitions and derives where their types match
that contract. Assess capture/load cost and complexity against the complete contract.
Array support alone is insufficient justification.

Validate a complete candidate before replacing the live machine. Restore its hardware
state, including all nonvolatile contents, and reconstruct derived scheduling without
advancing time. Failed loads leave the session intact. Frontends own files, slots,
compression, replacement, backups and the policy for writing restored EEPROM to their
ordinary save file.

Verify subsequent observations with identical inputs after captures during partial
instructions, serial shifts, timers and programming. The same capture mechanism can
support rewind or debugging when needed. Serialization incurs work when requested.

Any future interchange proposal must state whether it preserves exact subsequent
behavior or offers a usable session with reconstructed details. BESS makes that tradeoff
explicitly; HachiStep's native contract requires exact restoration.
[Save state research](research/emulator-state-and-testing.md).

## 14. Independent hardware conformance

[hachiware](https://github.com/lumirth/hachiware) owns diagnostic programs,
signal fixtures, expected observations, applicability and measurement records. It is a
separate repository with no dependency on `hs-core`. HachiStep's adapter loads and runs
cases through production execution and exports observations. The same cases should serve
another emulator or a physical test runner.

Local tests protect embedding behavior, save/restore, resource guarantees and concrete
regressions. Assertions should survive changes to private structures and helper
organization. Independent expectations come from documentation, observations, firmware
evidence or justified inference. Mooneye, SameSuite, mGBA and Dolphin provide relevant
examples in the
[testing research](research/emulator-state-and-testing.md#hardware-test-suites).

### 14.1 Suite structure

Organize cases by hardware mechanism. Keep a program's expected observations, duration,
conditions and evidence beside its definition. A module may generate related cases
from a table. Share instruction encoding and target definitions where they remove
duplication. Keep generated corpora and run reports in ignored output directories;
retain curated reference data with the cases that use it.

Adapters declare the observations and inputs they support and their configured
conditions. Each case requests the observations it needs. The adapter owns conversion
to its execution clock and verifies completion of the requested experiment. The suite
retains failed observations and records the inputs, adapter and executable identities.

Support guest diagnostics, component signal fixtures and CPU cases with an explicit bus
setup. State which environment each result describes.

### 14.2 Result basis

Each case records its identifier, target, distinguishing behavior, setup, expected
observations, evidence and limits. Report pass, fail, unknown expectation, not
applicable and runner failure distinctly. A total pass rate across those categories
would hide the meaning of the results.

## 15. Test construction

Measure the intended mechanism with a controlled guest sequence. Save CCR before
reporting changes it. Account for SLEEP's interrupt behavior, timer calibration and
debugger effects. Write results after the measurement window where possible. Gekkio's
guidance explains how test code can change the behavior it measures.
[Test ROM design][13].

### 15.1 Observations

Identify external pin captures, guest register observations, documented internal timing
and inferred internal sequencing accurately. The 8088 SingleStepTests corpus illustrates
useful evidence records; available Pokéwalker observations are constrained by its own
hardware. [SingleStepTests][14].

Use a small, relocatable RAM result area with a signature, case identifier,
running/completed state, first failing subcase and observed values. Write completion
last. The adapter reads this memory without guest side effects; a physical runner
retrieves the same data after measurement.

### 15.2 Coverage

Enumerate tractable spaces such as decode words, byte arithmetic, register aliases and
small control fields. Sweep accesses before, at and after relevant edges, with the
applicable enable, flag, clock, power and width settings.

Use sequences for read-qualified clears, interrupt deferral, sensor shadows, buffer
transfers, divider phase and nonvolatile commands. Combine mechanisms according to
actual connections, such as clocks with timers, GPIO with serial traffic and supply
changes with programming. This keeps the test set focused on reachable interactions.

Input generation and expected results need independent foundations. Validate analog and
timing models against observations beyond the data used to fit them.

## 16. Distinguishing conformance cases

These cases directly exercise architectural decisions that otherwise tend to be hidden
by ordinary retail operation.

| Test                                 | Controlled experiment                                                                    | What it distinguishes                                                          |
| ------------------------------------ | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| Exception stack order            | Observe the two stack destinations and interrupt/reset the sequence between accesses.    | Correct final frame versus correct physical write order.                       |
| Discarded fetch behavior         | Arrange controlled instruction-fetch observations around exception entry.                | Actual prefetch activity versus invented internal delay.                       |
| RTC update window                | Read each time register around its update phase; perform the two-snapshot sequence.      | Atomic host time versus actual busy/update behavior.                           |
| RTC deferred dispatch            | Run a workload that defers firmware time handling while RTC interrupts continue.         | Hardware execution versus high-level elapsed-time compensation.                |
| Watchdog reset identity          | Trigger watchdog reset without removing external-device power.                           | Correct reset domain and firmware-visible reset cause.                         |
| Timer clear race                 | Sweep a flag-clear access across the event that sets the flag.                           | Local conflict semantics versus arbitrary scheduler order.                     |
| Clock gate phase                 | Disable and re-enable a downstream module at several parent-clock phases.                | Retained divider phase versus restart-at-write behavior.                       |
| Serial holding versus shifting   | Queue data and sample status around holding-register and shift-register boundaries.      | Distinct hardware states versus one transfer-complete flag.                    |
| LCD parameter across deselection | Send command and parameter in separate complete-byte selected intervals.                 | Byte assembly lifetime versus command-parser lifetime.                         |
| BMA wake filter fill             | Apply a controlled input while waking at different bandwidth settings.                   | Actual filter startup versus a fixed delay followed by steady-state filtering. |
| Interrupted staged commit        | Interrupt an isolated storage workload at several accepted/committed write boundaries.   | Real persistent sequence versus atomic host transaction.                       |
| Waveform reconfiguration         | Change timer configuration midway through a pulse and compare reconstructed transitions. | Lossless signal representation versus frequency-only audio approximation.      |

The first group can use documented expectations where the manuals settle the result. A
measurement-dependent case stays in the research category until its expectation is
established.

Tests involving flash wear, persistent changes, or physical power interruption must be
explicitly classified. They should not silently run as part of an ordinary
non-destructive hardware test batch.

---

## 17. Verification of optimized representations

### 17.1 Execution partitions

Run the same scenario in one call, fixed short calls, random partitions and calls ending
around important effects. Compare canonical state and complete observable histories,
including a further interval after restoration. Repeat representative cases on native
and browser hosts with identical inputs.

### 17.2 Clock arithmetic

Compare bulk advancement with an independent rational calculation. Cover fractional
remainders, long intervals, source changes, gates, stabilization, restoration and epoch
rebasing.

### 17.3 Outputs and caches

Expand compact output descriptions and compare every transition. Exercise initial
transients, reconfiguration, simultaneous terminal changes and captures inside a period.
Different segment boundaries are equivalent when they reconstruct the same signal.

For decode caches, repeat with empty, warm and invalidated metadata. Include RAM
execution, changing extension words, instructions across boundaries, flash programming
and mutations on either side of a fetch.

### 17.4 State comparison

Normalize or project lazy representations without guest read effects before comparing
them. Independent recurrences and mathematical oracles belong in tests; production
retains the single execution mechanism.

## 18. Failure localization

Find the first divergent register read, interrupt, pin transition or persistent change.
Use `pw` to identify the firmware sequence exposing it, preserve the surrounding state,
reduce it to a hardware question and correct the owning mechanism. Keep the reduced
regression and the original workload.

Capture compact diagnostics near the divergence. Host tools handle symbols, source
annotation, disassembly and trace presentation. Disabled tracing adds no allocation,
formatting, atomic counters or observation calls to ordinary instruction effects. Expand
the minimal controls in §13.1 when an actual debugging task requires it.

## 19. Performance specification

Native and browser execution are target uses. Optimize portable algorithms, data
representations, and eliminated work. Do not introduce ARM64- or WebAssembly-specific
tricks or alternative execution paths; the compiler may perform its normal
target-specific lowering.

Interactive embedding shapes the public interface. Measure interactive use and batch
firmware execution as equally important workloads through the complete core.

### 19.1 Execution cost

Host work should follow CPU effects, necessary component evolution, interactions, input
changes and changes to output descriptions. Exact arithmetic can remove repeated
increments and signal expansion where hardware behavior permits it. Preserve the local
processing needed by changing sensor inputs or other histories.

| Operation | Required implementation property |
| --- | --- |
| Register instruction | Direct arithmetic and a local timing-budget update. |
| Ordinary memory access | Direct access at the physical width. |
| Peripheral access | Synchronize the affected owners. |
| Inactive peripheral | Revisit it when an input or control change requires work. |
| Counter evolution | Use exact arithmetic where applicable. |
| Serial transfer | Consume bounded runs of edges through the canonical shifter. |
| Stable buzzer output | Describe its waveform compactly. |
| LCD mutation | Update controller state; convert pixels on request. |
| Sleep | Advance to the next relevant consequence. |

### 19.2 Memory

Construction may reserve backing storage. Ordinary execution allocates nothing. Measure
production and generated code, immutable tables, retained hardware state and disposable
metadata. Include generated executable size in specialization costs.

### 19.4 Benchmark matrix

Anchor optimization decisions in realistic Pokéwalker execution: retail workloads and
plausible custom firmware. Synthetic cases are useful for isolating costs and exercising
mechanisms, but a large synthetic speedup does not establish a meaningful improvement
for the project. Verify the benefit in the relevant realistic workload.

The performance corpus must include:

| Workload                                  | Reason                                                       |
| ----------------------------------------- | ------------------------------------------------------------ |
| Cold construction and first execution     | Exposes hidden predecode/setup cost.                         |
| Custom firmware with frequent register operations and branches | Measures CPU execution overhead.                             |
| RAM execution and self-modification       | Prevents immutable-code-only optimization.                   |
| RTC stable-read polling                   | Exercises fine timing and repeated register synchronization. |
| EEPROM status polling and page bursts     | Separates serial and programming costs.                      |
| Motion acquisition and processing bursts  | Exercises real CPU/peripheral interaction.                   |
| Display command and data traffic          | Measures controller versus presentation work.                |
| Sound with concurrent activity            | Tests output compression without lost feedback.              |
| Infrared traffic                          | Measures bit-level timing and buffering.                     |
| Long inactive intervals                   | Detects unnecessary periodic work.                           |
| Very short run horizons                   | Measures resumability overhead.                              |
| Mixed whole-device sessions               | Prevents optimizing only synthetic best cases.               |

Measure cold and warm behavior separately.

### 19.5 Measurement and acceptance

Pin the source revision, compiler, build settings, target, inputs, physical parameters
and output consumer. Use repeated paired runs with varied ordering and report their
spread. Measure wall/CPU time, memory, allocation count, compiled code and table size,
short-call latency and active throughput. Report energy only when measured.

Comparisons must identify differences in modeled behavior and enabled output. A faster
approximate competitor still provides a useful performance target. An accepted
optimization identifies the repeated work removed, the invariant that permits it, the
independent checks, workload gains and regressions, and its code/memory cost. The
default build uses the selected implementation with its full hardware behavior.

## 20. Completion criteria

Measure performance and test accuracy as each mechanism becomes executable. Completion
requires its accesses, retained state, timing, reset/power behavior, interactions and
applicable corner cases to be accounted for. Expectations need an independent basis.
Each retained state variable and abstraction should explain hardware behavior or remove
demonstrated host cost.

[1]: https://dmitry.gr/?proj=28.+pokewalker&r=05.Projects "https://dmitry.gr/?proj=28.+pokewalker&r=05.Projects"
[2]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual "https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual"
[3]: https://archive.ares-emu.net/near.sh/articles/design/cooperative-threading.html "https://archive.ares-emu.net/near.sh/articles/design/cooperative-threading.html"
[4]: https://docs.mamedev.org/techspecs/cpu_device.html "https://docs.mamedev.org/techspecs/cpu_device.html"
[5]: https://mgba.io/2017/04/30/emulation-accuracy/ "https://mgba.io/2017/04/30/emulation-accuracy/"
[6]: https://floooh.github.io/2019/12/13/cycle-stepped-6502.html "https://floooh.github.io/2019/12/13/cycle-stepped-6502.html"
[7]: https://github.com/cbiffle/rs80 "https://github.com/cbiffle/rs80"
[8]: https://rodrigodd.github.io/2023/09/02/gameroy-jit.html "https://rodrigodd.github.io/2023/09/02/gameroy-jit.html"
[9]: https://www.st.com/resource/en/datasheet/m95512-w.pdf "https://www.st.com/resource/en/datasheet/m95512-w.pdf"
[10]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf "https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf"
[11]: https://sameboy.github.io/posts/release-0.15 "https://sameboy.github.io/posts/release-0.15"
[12]: https://github.com/Gekkio/mooneye-test-suite "https://github.com/Gekkio/mooneye-test-suite"
[13]: https://gekkio.fi/blog/2016/game-boy-test-rom-dos-and-donts/ "https://gekkio.fi/blog/2016/game-boy-test-rom-dos-and-donts/"
[14]: https://github.com/SingleStepTests/8088 "https://github.com/SingleStepTests/8088"
