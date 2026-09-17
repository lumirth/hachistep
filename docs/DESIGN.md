# HachiStep — implementation design specification

This specification and the supplied starter are the approved starting point. Design review should still challenge their assumptions through concrete consequences and revise choices when warranted. The clarifications below incorporate the design interview.

**HachiStep should be a statically composed model of one physical machine, executed by one host-compiled, resumable interpreter. Its main optimization is to represent hardware evolution compactly—not to execute a cheaper approximation of it.**

The implementation should distinguish three things that are often unnecessarily conflated:

| Thing being represented                    | Appropriate representation                                                                           |
| ------------------------------------------ | ---------------------------------------------------------------------------------------------------- |
| CPU operations and hardware interactions   | Ordered, timed state transitions.                                                                    |
| Predictable evolution between interactions | Counter arithmetic, retained clock phase, bounded histories, and other exact update formulas.        |
| Repetitive externally visible signals      | Lossless signal descriptions, expanded only when a consumer needs individual transitions or samples. |

A timer producing a stable waveform does not inherently require thousands of global scheduler events. A CPU instruction does not inherently require an allocated micro-operation program. A hardware register does not inherently require a generic register object. None of those reductions permits omitting a behavior that firmware or another component can observe.

The governing invariant is:

> **Given the same initial physical state and input history, changing how much host work is grouped together must not change any hardware effect, its timing, or the state determining future effects.**

This specification selects the architecture and its implementation contracts. “Fastest” remains a measured property of the resulting default build, not something an architecture can establish by assertion.

---

## 1. Target and fidelity contract

### 1.1 Model a physical Pokéwalker, not a firmware workload

Model the actual Pokéwalker hardware, with explicit physical conditions including power transitions. Introduce hardware-revision distinctions only when evidence establishes a real difference.

The target comprises the H8/38606 MCU, its connected peripherals and board wiring, the external EEPROM, accelerometer, display controller, infrared circuitry, buttons, buzzer, and relevant supply behavior. Public board research identifies the H8/38606, M95512 EEPROM, and BMA150 accelerometer; Renesas’s target-specific addition establishes the 48 KiB flash and 2 KiB RAM configuration. ([Dmitry.GR][1])

The main hardware sources are:

| Question                                             | Authority                                                                   |
| ---------------------------------------------------- | --------------------------------------------------------------------------- |
| Target-specific memory and flash differences         | H8/38606 addition and applicable corrections.                               |
| CPU instruction semantics                            | H8/300H software manual, restricted by the actual MCU.                      |
| MCU timing, registers, clocks, interrupts, and power | H8/38602R hardware manual with target-specific amendments.                  |
| External-chip behavior                               | The applicable chip and revision’s datasheet, supplemented by measurements. |
| Which chip pins are connected                        | Board evidence.                                                             |
| What firmware does with those connections            | `pw` and observed execution.                                                |

A comment in reconstructed firmware is evidence of an interpretation, not automatically a silicon specification. The actual instructions and register accesses are stronger evidence of software behavior.

The MCU and CPU manuals explicitly divide instruction semantics from target-specific hardware behavior. Both are necessary; implementing a generic H8 instruction list is not sufficient. ([Renesas][2])

**Evidence and inference.** Documentation, physical observations, and `pw` provide complementary evidence. `pw` is the matching firmware decompilation and a strong reference for what the real device actually executed successfully. Use its complete instruction sequences, surrounding state, and observed outcomes to constrain the hardware model. Infer the hardware mechanism that explains them, so custom firmware benefits too.

Before declaring a behavior undocumented or unresolved, search the applicable manuals, target additions, corrections, related sections, component documentation, and relevant firmware behavior thoroughly. A search failure is not evidence that documentation does not exist. Reconcile conflicting sources; a datasheet is not infallible.

Strong, coherent inference is a sufficient basis for implementation. Implement the best-supported behavior and refine it when stronger evidence appears. Lack of absolute certainty or an explicit datasheet sentence is not by itself a reason to stop execution, leave functionality unimplemented, or add user-facing warnings. Keep a concise rationale and a focused regression or distinguishing experiment where it helps future work; uncertainty is not a product feature.

### 1.2 What must remain indistinguishable

The relevant observations include more than final registers and screenshots:

| Observation               | Required fidelity                                                                         |
| ------------------------- | ----------------------------------------------------------------------------------------- |
| Memory and register reads | Correct value, width, access timing, and side effects.                                    |
| Interrupts and reset      | Correct generation, sampling, admission, priority, entry sequence, and interrupted state. |
| Connected signals         | Correct levels, drive state, edges, and timing relationships.                             |
| Display                   | Correct controller behavior and resulting display history.                                |
| Sound and infrared        | Correct signal history, not substituted named sounds or decoded packets.                  |
| Persistent storage        | Correct accepted operations, progress, completion, and power/reset interactions.          |
| Future execution          | All hidden state capable of affecting later behavior is retained.                         |

The core must support arbitrary firmware through these same mechanisms. Firmware hashes, function addresses, familiar polling loops, or known application states must not select different behavior.

### 1.3 Physical variation is not an accuracy setting

Three categories must remain separate:

**Hardware rules** belong in the implementation: instruction effects, command grammar, latch behavior, and register semantics.

**Unit characteristics** describe an actual or canonical physical unit: clock frequencies, calibration values, and characterized analog parameters.

**Unresolved behavior** is a research limitation. It must not become a public collection of arbitrary “maybe the hardware works this way” switches.

A deterministic emulator can reproduce a chosen physical realization and reproducible noise process. It cannot honestly call arbitrary startup values or guessed analog behavior universal properties of every manufactured unit.

The default must use the complete model. Selecting a different characterized unit must change physical parameters, not enable omitted hardware.

Identical firmware, persistent contents, physical configuration, and timestamped
inputs must produce identical hardware observations on native and browser hosts
running the same core revision. Default construction is reproducible. Any modeled
noise or unit variation starts from explicit, reproducible initialization; its
future-determining state belongs in save states. Host clocks, ambient randomness,
and run-call partitioning must not introduce variation.

---

## 2. The fundamental execution model

### 2.1 Separate ownership from synchronization

**Ownership** answers where the authoritative state resides.

**Synchronization** answers when one owner can affect another owner’s next result.

Those are different maps.

A timer owns its counter, compare registers, flags, and output state. It does not follow that every counter increment needs to enter the global scheduler.

An EEPROM owns its command parser and programming state. It does not follow that an entire serial byte can be delivered atomically.

The useful precedent is demand-driven synchronization: Near describes synchronizing components when they communicate rather than continuously switching between them. The same account identifies the overhead and serialization problems of stackful execution. HachiStep should adopt the synchronization principle without adopting a thread or coroutine stack for every component. ([Near Archive][3])

### 2.2 The production mechanisms

The core needs five principal mechanisms:

| Mechanism               | Responsibility                                                          |
| ----------------------- | ----------------------------------------------------------------------- |
| Timed CPU executor      | Executes instruction effects and retains necessary continuation state.  |
| MCU access authority    | Resolves addresses, physical widths, and access timing.                 |
| Clock model             | Preserves source phase, divider relationships, gating, and transitions. |
| Boundary scheduler      | Finds the next interaction that cannot be crossed.                      |
| Fixed board connections | Propagates actual pin, interrupt, and device relationships.             |

These should be concrete code, not a plugin framework.

There is no need for a universal device interface, effect-dispatch virtual machine, asynchronous task runtime, registered callback graph, or dynamically allocated event queue.

### 2.3 What “one engine” means

The production implementation must not contain:

```text
Whole-instruction executor + partial-instruction executor
Interpreter + cached-block runner
JIT + interpreter fallback
Retail firmware substitutions
Speculative execution + state rollback
Accurate serial path + atomic-byte shortcut
```

Ordinary distinctions remain legitimate: RAM versus a register, a running versus stopped clock, different instructions, or advancing one versus several hardware edges.

The test is whether there are **two implementations of the same guest-visible effect**.

MAME documents paired normal/restarted instruction implementations as a performance technique, including for H8. That is useful prior art, but it is not the selected organization here. HachiStep should use one resumable implementation rather than ship both forms. ([MAME Documentation][4])

### 2.4 Do not confuse fewer scheduler visits with fewer hardware effects

A component can process several transitions together when it preserves their complete consequences.

For example, a counter can advance by arithmetic. A shift register can consume an exact run of bits. A stable timer output can be described by its period and phase.

These are compact representations of the same evolution—not permission to omit it.

mGBA’s accuracy discussion makes the important distinction between correct total cycle counts and correct ordering of interactions within those cycles. HachiStep requires the latter, even when host work is grouped. ([mGBA][5])

---

## 3. Code organization and authority

### 3.1 Production layout

Use one core crate, with modules organized around actual mechanisms:

```text
core/
  machine.rs        Fixed composition, boundary coordination, public operation
  time.rs           Timestamp and clock-phase arithmetic
  signals.rs        Small electrical and lossless-output value types

  cpu/
    state.rs        Registers, fetch state, continuation, interrupt admission
    decode.rs       Instruction classification and extraction
    execute.rs      The single execution loop
    alu.rs          Explicit-width arithmetic and flag helpers
    forms/          Readable instruction-family definitions

  mcu/
    bus.rs
    clocks.rs
    reset.rs
    interrupts.rs
    gpio.rs
    timer_b1.rs
    timer_w.rs
    rtc.rs
    watchdog.rs
    aec.rs
    sci.rs
    ssu.rs
    iic.rs
    adc.rs
    comparators.rs
    flash.rs

  devices/
    bma150.rs
    m95512.rs
    nt7508.rs
    infrared.rs
```

These are responsibility boundaries, not a requirement that every item be a separate file regardless of size.

Split a module when it separates a substantial mechanism or makes navigation easier. Do not split every register, instruction, or state transition into its own file.

### 3.2 Authority rules

| Question                                 | Sole authority                                         |
| ---------------------------------------- | ------------------------------------------------------ |
| What does an instruction do?             | Its CPU implementation.                                |
| When does an instruction’s effect occur? | Its timed sequence and the applicable access contract. |
| Which owner receives an address?         | MCU address routing.                                   |
| What does a register access mean?        | The owning peripheral.                                 |
| Which source drives a package pin?       | GPIO/function selection.                               |
| Which pins share a net?                  | Board composition.                                     |
| What happens during simultaneous causes? | The affected hardware owner’s conflict rule.           |
| When is the next consequence?            | Derived from owner state; cached by the scheduler.     |
| How are outputs rendered or stored?      | Host adapters, without changing hardware execution.    |

A scheduler is not the authority for timer semantics. A bus is not the authority for EEPROM commands. A frontend is not the authority for RTC progression.

### 3.3 One authoritative state, not one representation everywhere

Do not duplicate mutable hardware truth:

```text
No raw SFR array shadowing separately mutable peripherals.
No frontend-owned framebuffer standing in for controller RAM.
No second EEPROM image that commits independently of the chip model.
No independently maintained copies of the same interrupt flag.
```

However, retain distinct states that physically exist:

```text
Transmit holding register and transmit shift register.
Sensor nonvolatile image and working configuration.
EEPROM page buffer and committed cells.
Programmed display setting and latched scan state.
```

Calling two fields “redundant” does not make them redundant.

### 3.4 Dependencies point toward integration

Leaf components may use time arithmetic and small signal types. They must not import product run-loop types or frontend services.

The integration layer should:

1. Invoke an owner.
2. Receive a small description of changed connections or scheduling requirements.
3. Update the fixed set of affected neighbors.

A compact bitmask for “these connections changed” is appropriate. An allocated list of generic semantic effects is not.

Use ordinary borrowing over disjoint state. Do not solve ownership with `Rc<RefCell<...>>`, shared globals, per-chip locks, or cloning the machine around each operation.

### 3.5 What may be shared

Good shared mechanisms include clock-phase arithmetic, fixed-width arithmetic helpers, and small serial-shift operations.

Do not assume that all peripherals share the same:

* Flag-clear rules.
* Reset semantics.
* Serial transaction lifetime.
* Timer behavior.
* Nonvolatile programming procedure.

A shared abstraction should follow demonstrated identical behavior, not visual similarity in the datasheets.

### 3.6 Rust implementation policy

Keep dependencies few and justified by the total complexity they remove. Zero
dependencies is not a goal in itself. Evaluate core runtime, build-time, test,
and frontend dependencies separately; adding one requires an actual need.

Use `std` for the initial Rust core. Keep files, host clocks, threads, and platform
services outside hardware execution. Revisit `no_std` when an actual consumer
requires it; no extra target or configuration matrix is needed now. Construction
and explicit save-state operations may allocate; ordinary execution must not.
The source comparison and concrete dependency recommendations are recorded in
[emulator dependencies](research/emulator-dependencies.md).

Unsafe Rust is forbidden by default. An exception requires a concrete,
proportionate justification: a meaningful measured benefit or substantially less
complexity than the viable safe alternatives. Convenience alone is insufficient.
Any exception must be narrow, explain its safety invariants, and preserve the
portable single execution mechanism. Do not weaken the existing prohibition
before such a case exists.

This prohibition applies to HachiStep's own code. Calling safe APIs from the
standard library or a justified dependency does not require their internals to
contain no unsafe code.

---

## 4. Time: clock-local execution, high-precision boundaries

### 4.1 Do not derive the whole machine from one convenient nominal clock

The authoritative clock representation should be **source phase and edge ordinal**.

A CPU clock, an independently running sensor oscillator, and a watch source must not be forced into accidental phase relationships because their nominal frequencies happen to fit an integer lattice.

Use:

```text
Clock source:
    frequency representation
    epoch
    edge ordinal
    retained fractional phase

Derived clock:
    source
    divider state
    gate position and state
    reset/hold behavior
```

Shared prescalers remain shared. Gating after a divider is not the same as stopping the divider.

### 4.2 Selected timestamp representation

Use an opaque **64.64 fixed-point timestamp in seconds**, represented by `u128`.

Its resolution is:

$$
2^{-64}\text{ seconds}.
$$

This is numerical precision, not a claim that the physical clocks are known to that accuracy.

The wider timestamp belongs at component and API boundaries. The CPU hot loop should use a **64-bit local clock-progress budget**, not perform wide time conversion after every instruction.

This combination provides:

* A common timeline for independently characterized frequencies.
* Exact shared-clock relationships through edge ordinals.
* No floating-point drift.
* Cheap ordinary CPU progress.
* Explicit, bounded timestamp quantization.

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

For single-period advancement, this reduces to addition and an occasional carry. Bulk advancement needs division only at the synchronization boundary.

The arithmetic must use checked operations. Construction validates frequency representations; impossible arithmetic is a host API/model error, never silent wraparound.

### 4.4 CPU waits are clock obligations

A CPU wait should describe remaining clock progress, not merely an immutable wall-time deadline.

If the source changes, stops, or undergoes stabilization, the remaining obligation must be interpreted through the new clock state without inventing elapsed cycles.

The same applies to serial dividers and conversions.

Do not restart every affected peripheral at the timestamp of a clock-control write. Preserve the phase that the hardware preserves.

### 4.5 External horizons

`run_until(T)` should stop before effects at `T`.

An exact horizon is selected because it permits deterministic input replay and causal coupling between devices without speculative execution. It must not require a host iteration for every intervening clock tick.

The caller supplies inputs only through a known-valid horizon. The core must not advance past unknown live input and later insert an event into its own past.

---

## 5. Scheduling: advertise consequences, not clocks

### 5.1 The component obligation

Every independently evolving mechanism must be able to answer:

> What is the earliest future instant at which my evolution could affect another owner or produce an output change that cannot remain represented compactly?

That is its next scheduling obligation.

Examples include an interrupt assertion, a conversion completing, a serial receiver sampling a changed signal, or a nonvolatile operation completing.

A counter increment that only affects a later register read need not be a global appointment.

### 5.2 Calendar organization

Use fixed appointment slots for the machine’s finite set of independently reschedulable mechanisms.

Maintain a cached minimum.

* Moving a non-minimum appointment later does not require a complete rescan.
* Moving an appointment earlier may replace the minimum.
* Cancelling or postponing the minimum requires recomputation.
* Due slots at one timestamp are collected together.

Do not allocate an event object for every transition.

This is a suitable concrete implementation for a fixed machine, not a claim that a fixed array beats every other calendar on every host.

### 5.3 The execution loop

The coordination algorithm is:

```text
Determine the earliest external/input/peripheral boundary.

Run the CPU's single implementation toward that boundary.
    Register-only work stays inside the CPU loop.
    A peripheral access synchronizes only its relevant owners.
    A change that invalidates the boundary updates the execution budget.

At the boundary:
    collect simultaneous causes;
    resolve affected hardware;
    propagate connected signals;
    update affected appointments;
    publish resulting output changes.

Repeat until the caller's horizon.
```

There must not be a full peripheral sweep between instructions.

### 5.4 Synchronize before changing a rule

A peripheral configuration write must perform these operations in order:

```text
Materialize relevant evolution under the old configuration.
Resolve activity coincident with the write.
Apply the accepted register change.
Preserve/reset phase according to that register's rule.
Recalculate affected future consequences.
```

The order matters.

For example, lowering a compare value must not retroactively apply the new period to elapsed time.

### 5.5 Same-time behavior is local hardware behavior

A universal rule such as “run all devices, then perform CPU writes” is insufficient.

SameBoy’s CPU implementation contains register-specific conflict handling, including distinctions between old-value reads, new-value reads, CPU-write precedence, and transient register behavior. That is evidence that correct synchronization needs more than a global event ordering convention.

For HachiStep, each affected owner receives the relevant simultaneous causes and resolves them according to its hardware rule.

An internal transient lasting a real clock phase must be represented as such. Comparing only the initial and final register values can erase a real pulse or latch effect.

### 5.6 Never reuse an invalidated boundary

After any operation that can change wake eligibility, clock availability, or an appointment, re-evaluate the relevant boundary before advancing again.

NanoBoyAdvance’s halt loop explicitly rechecks whether the CPU should wake after DMA activity before advancing to the next scheduled event. HachiStep has different hardware, but the synchronization lesson is directly applicable: one action can invalidate the assumption used to choose the next advance.

### 5.7 No rollback

A missed boundary is a model defect.

Do not conceal it by restoring a CPU snapshot and replaying the instruction in another mode. The architecture must prevent crossing consequential boundaries in the first place.

---

## 6. Exact compressed advancement

### 6.1 One advancement function, including the one-edge case

For a mechanism that admits analytical advancement, implement one function over an elapsed edge count.

Advancing one edge is simply \(n=1\).

Do not maintain a tick implementation and a separately maintained “bulk” implementation in production.

A simple tick recurrence can exist in the test project as an independent mathematical oracle.

### 6.2 Exclusive edge counting

Given next unprocessed edge \(e\), period \(P\), and exclusive endpoint \(T\):

$$
n=
\begin{cases}
0,&T\le e,\\[2pt]
1+\left\lfloor\frac{T-1-e}{P}\right\rfloor,&T>e.
\end{cases}
$$

The formula applies within an integral local clock coordinate. Conversion between independent clocks is handled separately.

### 6.3 Counter example

For modulus \(M\), current count \(c\), and \(n\) increments:

$$
c'=(c+n)\bmod M,
\qquad
o=\left\lfloor\frac{c+n}{M}\right\rfloor.
$$

The overflow count \(o\) is not necessarily the output.

The owner must determine whether those overflows:

* Set an already-set sticky flag.
* Change an interrupt line.
* Toggle an output.
* Advance another counter.
* Trigger some other state transition.

Only consequences that are genuinely redundant may disappear from the global schedule.

### 6.4 Histories cannot be discarded merely because nobody read them

A filter, synchronizer, qualification counter, or serial parser may retain history that affects future behavior.

Deferring a sensor read must not replace an entire interval of motion with the latest host sample.

Where no exact compression is established, execute the necessary local transitions. That is still the same model; it does not justify stepping unrelated hardware clocks.

### 6.5 The compositional requirement

For constant configuration and an appropriately fixed input segment, advancement must satisfy:

$$
A(A(s,a),b)=A(s,a+b)
$$

after canonicalization of equivalent lazy representations.

This identity is a concrete test obligation for each compressed mechanism. It protects against lost fractional phase, repeated rounding, and call-size-dependent filter or counter behavior.

---

## 7. CPU execution without an execution framework

### 7.1 State

The CPU needs:

```text
Architectural registers and CCR
Instruction-fetch/prefetch state
One continuation identifier
Operands still needed by that continuation
Temporary values that must survive suspension
Current physical access, if any
Remaining clock obligation
Interrupt-admission state
```

It does not need a full cloned “before” CPU, a full “after” CPU, a runtime capability object, and an allocated instruction plan.

### 7.2 One compiled continuation program

Use a flattened continuation identifier that selects the next timed effect.

The implementation may execute several effects directly before returning to machine coordination. It stores a continuation only where suspension can actually occur.

Floooh’s cycle-stepped CPU work demonstrates the practicality of generated flat state machines with explicit pin behavior. The selected H8 design uses that organization while advancing across non-interacting time spans rather than universally iterating every clock tick. ([Floooh][6])

### 7.3 Readable source, flat generated execution

Instruction source should be organized by semantic family. A small build-time generator may mechanically produce:

```text
Decode classification
Instruction-form specialization
Continuation identifiers
Direct dispatch bodies
```

An instruction definition needs:

| Field                              | Purpose                                               |
| ---------------------------------- | ----------------------------------------------------- |
| Encoding and selector restrictions | Classify actual fetched bits.                         |
| Operand extraction                 | Registers, widths, immediate and displacement fields. |
| Ordered physical accesses          | Preserve the real access sequence.                    |
| Internal computation               | Direct host arithmetic.                               |
| Commitment points                  | Specify when effects become visible.                  |
| Clock obligations                  | Distinguish access timing from internal time.         |
| Interrupt-admission behavior       | Preserve instruction-specific exceptions.             |
| Source reference                   | Identify the hardware basis.                          |

The generator must not become a general optimizer or a new programming-language implementation.

It must not generate both a whole-instruction and a restarted executor.

It must not generate its independent expected test results from the same semantic bodies.

Prefer build-time generation and data-first descriptions for repetitive structures that would otherwise need extensive handwritten logic. Generate deterministically from version-controlled inputs; ordinary execution does not depend on running the generator. Account for both the generator maintenance burden and the generated footprint.

### 7.4 Do not charge twice for precision

A CPU effect should not pass through all of these layers:

```text
Decoded instruction
→ capability classification
→ semantic operation object
→ effect vector
→ transaction plan
→ physical access interpreter
```

Decode once into the facts needed for execution. Perform direct arithmetic directly. Route a physical access through one bus authority.

Precision requires retaining the right state and ordering—not repeatedly re-describing the operation.

### 7.5 Arithmetic and register representation

Use one authoritative register file.

Register aliases are masks and shifts over that file, not separately mutable byte, word, and long arrays.

Every operation uses explicit target widths. Host signed overflow, host division traps, and host shift behavior must not determine guest behavior.

Instruction-specific flag updates should be explicit. A generic helper that always rewrites every arithmetic flag is inappropriate for instruction families that preserve some flags.

Do not initially add a lazy-CCR subsystem. It adds another representation of architectural state and complicates asynchronous exits. Introduce it only if measured savings justify a complete materialization contract.

### 7.6 Specialization has a footprint cost

Specialize instruction facts that eliminate meaningful runtime work, particularly width and semantic family.

Do not blindly generate every register-selector combination for a large encoding space.

`rs80` demonstrates useful opcode specialization and state-layout work, but also documents that more elaborate threaded dispatch did not improve its tested implementation. Its results support profiling generated code, not declaring one dispatch technique universally superior. ([GitHub][7])

### 7.7 Decode caches do not execute code

The default should use a compact decoder feeding the single executor.

A decode cache may be added when it gives a reproducible improvement. It stores interpretation of fetched bits, not a different program runner.

A cache must not bypass:

```text
Hardware fetching
Fetch timing
Side-effectful fetches
Prefetched bytes already in the CPU
RAM execution
Extension-word changes
Internal flash programming
```

Clearing all decode metadata must change only host runtime.

### 7.8 Why the design does not select a JIT

A JIT is not inherently inaccurate. The problem here is the additional compiler, code-cache lifecycle, host-specific behavior, and precise-exit machinery required to keep one complete execution implementation.

GameRoy’s published design combines lazy component updates with JIT compilation, but returns to interpretation near timing boundaries. Its measured results also show substantial gains from peripheral work reduction independent of native-code generation. That architecture is informative, but its fallback arrangement does not meet this specification. ([Rodrigodd][8])

The selected implementation is host-compiled timed interpretation. Experimental replacements must replace the engine, not accumulate beside it as additional production modes.

---

## 8. CPU timing must describe effects, not just totals

### 8.1 Physical accesses are separate from instruction completion

A multi-access instruction must retain the completed prefix of its effects.

If the first access occurs and reset arrives before the second, the first remains completed. A future invalid or unsupported lane must not suppress an earlier physical access.

Resolve and execute accesses at their actual phases. Do not prevalidate a whole instruction as an atomic transaction.

### 8.2 Exception entry provides a concrete implementation test

The H8 hardware manual’s exception diagram shows actual bus activity, including discarded instruction prefetches and ordered stack writes. In the illustrated normal-mode sequence, the return PC is written at the old stack pointer minus two before the CCR word is written at the old stack pointer minus four. The same final stack bytes written in the opposite order are not the same hardware behavior. ([Renesas][2])

The exception continuation therefore needs separate stages for:

```text
Admission and saved-state capture
Required prefetch/internal activity
First stack write
Second stack write
Vector read
Handler fetch activity
Entry completion
```

Do not implement exception entry as a host four-byte store followed by `pc = vector`.

### 8.3 Interrupt generation and admission are different

Keep separate:

```text
Peripheral condition
Peripheral status latch
Controller request
Enable state
CPU mask
Request sampling/admission
Exception entry
```

A host suspension does not grant an interrupt opportunity.

Likewise, recomputing a single `pending && enabled && !masked` expression at retirement is insufficient when the target has already sampled relevant state.

The target documents special interruption behavior for `EEPMOV.B` and `EEPMOV.W`; they cannot share a generic “interrupt at each copied byte” rule. ([Renesas][2])

### 8.4 Undefined behavior is not permission to guess familiar behavior

Do not assume an unassigned address returns the last bus value, zero, or `0xFF`.

Do not infer that every address overflow wraps safely because the bus address is finite.

Where the target does not guarantee behavior, distinguish a measured result, a provisional model rule, and an unresolved question. None is an automatic guest exception unless the hardware actually produces one.

---

## 9. Memory and register access

### 9.1 One address decoder

The MCU bus should directly classify the fixed memory regions and implemented registers.

There is no need for a dynamically constructed memory-map framework.

Ordinary RAM and flash accesses should resolve to direct array operations. Register accesses should resolve to the owning peripheral.

### 9.2 Preserve width

Carry the physical width through the entire access.

Do not automatically implement:

```text
read16(address) = read8(address) + read8(address + 1)
```

That is wrong whenever the target has native word accesses, paired latches, width restrictions, or different access timing.

Similarly, a longword may require multiple physical accesses whose ordering matters.

### 9.3 Access state

One in-flight access needs only:

```text
Address and physical target
Read/write and width
Relevant access origin
Pending write data
Completed partial read data
Current physical phase
Remaining clock obligation
```

“Relevant origin” is narrow. Include instruction qualification where a peripheral distinguishes it; do not attach a general instruction-history object to every bus operation.

### 9.4 Register contracts

Each register or coupled group must have one explicit implementation contract:

| Aspect            | Required answer                                                          |
| ----------------- | ------------------------------------------------------------------------ |
| Storage           | Latch, counter projection, pin level, captured value, or derived status? |
| Width             | Which physical accesses are legal?                                       |
| Read              | What value is sampled, and when?                                         |
| Read side effect  | What changes because the read occurred?                                  |
| Write             | Which bits are accepted under current conditions?                        |
| Write side effect | Which operations, signals, or appointments change?                       |
| Conflict          | What happens with a simultaneous hardware event?                         |
| Reset             | Which reset domains affect it?                                           |
| Power             | Does it run, hold, reset, or become inaccessible?                        |

Addresses and dispatch belong in one routing authority. Masks and semantics belong with the owner. Do not maintain a second prose-derived runtime registry duplicating both.

### 9.5 Inspection is not a guest access

Debug inspection must not clear a flag, release a sensor shadow, acknowledge received data, or start another transfer.

It may project lazy state to the requested current instant. It must not call the side-effectful guest read function.

---

## 10. Required peripheral models

The following defines the implementation content of each owner. It is not an invitation to create a uniform base class.

### 10.1 Clocks, reset, and power

Represent each real reset and power domain separately.

A reset operation must specify:

```text
Which execution is aborted
Which latches reset
Which clocks continue
Which phases are retained
Which pins change drive
Which external chips see those pin changes
Which nonvolatile operations continue
```

Do not globally clear the scheduler when resetting the MCU. An external chip may still have an operation in progress.

`pw` makes reset cause externally consequential: startup checks the watchdog-reset indication and increments an EEPROM diagnostic counter. It also performs its own initialization and restoration sequence. A universal “reconstruct all objects from defaults” reset would bypass that distinction.

### 10.2 Interrupt controller

The controller owns its actual registers and controller latches. Peripheral flags remain peripheral-owned.

An eligible-request mask can be cached, but it is derived.

Recompute only affected eligibility when a source, enable, mask, route, or power condition changes.

Do not protect firmware from real read–modify–write losses. If a hardware flag can be cleared by a particular access sequence, reproduce that sequence’s consequences.

### 10.3 Timer B1

Retain counter/load distinction, selected clock, phase, control state, and request behavior.

Use exact reload arithmetic, including the initial partial traversal before the first reload.

A stopped load, running load, mode change, and source change are separate operations. They must not all call `restart_period_from_now()`.

### 10.4 Timer W

Retain:

```text
Counter and clock phase
General registers
Compare/capture roles
Buffer relationships
Output latches
Capture input history
Status/clear qualification
Interrupt controls
```

Compute the next relevant compare, overflow, capture, buffer-transfer, or output consequence.

The first cycle after a register change may differ from the eventual periodic cycle. Preserve that transient instead of immediately reducing the counter modulo a newly programmed period.

Timer outputs can be represented by exact signal laws when stable; the flags and capture mechanisms remain fully modeled.

### 10.5 RTC

Do not represent the RTC as a host calendar object incremented once per second.

Retain its divider phase, register-update state, busy behavior, raw register values, control state, and periodic conditions.

`pw`’s `RtcReadStable` waits for each time register’s busy bit, takes two complete seconds/minutes/hours snapshots, and retries until they agree. The firmware therefore explicitly depends on more than a single atomic time value.

The RTC may advance arithmetically across quiet intervals, but its projection at an access must preserve the update phase and the values visible at that phase.

Do not compensate for elapsed time by directly updating retail software counters. In `pw`, minute/hour work is deferred during infrared activity and repeated pending updates can coalesce. Executing the actual firmware matters.

### 10.6 Watchdog and asynchronous event counter

These are separate owners.

The watchdog retains qualification/protection state as well as its counter and reset behavior.

The asynchronous event counter retains its actual external-edge and pulse-width state. It must consume routed pin activity, not a host-provided “number of events.”

Neither should be implemented as a collection of modes inside Timer B1.

### 10.7 SCI and infrared

Keep holding registers, shift registers, frame position, clock phase, receiver sampling, error state, and status-clear qualification distinct.

The signal chain is:

```text
SCI logical transmission
→ IrDA pulse transformation
→ transmitter behavior
→ incident optical signal
→ receiver behavior
→ SCI sampling and framing
```

A packet adapter may produce an input waveform. It must not place bytes directly into the receive register.

Malformed pulses, partial frames, disablement during transmission, receive overrun, and clock changes must use the same model as ordinary traffic.

### 10.8 SSU and IIC

These owners must obey the actual function-selection and shared-resource rules.

Hardware serial operation and GPIO bit-banging must reach the **same resolved board signals and external-device logic**.

IIC requires open-drain behavior, start/stop recognition, acknowledgement, arbitration, and clock-stretching relationships. It is not SPI with another parser.

### 10.9 ADC and comparators

The ADC retains the sampled input separately from conversion progress and the final result.

An input change after sampling must not alter the already-held sample.

Comparators react according to their own continuous-input and response model. They are not periodically sampled ADC shortcuts.

Supply, temperature, and analog inputs must be explicit conditions. There must be no firmware-specific “battery low” value injected into a register.

### 10.10 Internal flash

Retain programming/erase/verification state and the applicable access restrictions, not just a writable array.

Fetches during programming must follow the same CPU and bus model.

Changing flash invalidates interpretation metadata for affected bytes, but must not retroactively alter bytes already fetched by the emulated CPU.

### 10.11 M95512 EEPROM

Separate:

```text
Serial bit state
Command/address state
Write-enable and protection state
Pending page/status data
Programming progress
Committed array and persistent status
```

The M95512 family documents 128-byte pages and variant-specific capabilities; features exclusive to an identification-page variant must not be assumed for the installed part. ([STMicroelectronics][9])

Clocking write data into a page buffer is not a persistent commit.

Status polling must observe the appropriate programming state at the actual serial sampling/latching points.

Power loss must not be given invented all-old or all-new atomicity.

`pw`’s staged-walk update sets a recovery marker, performs a long page-copy sequence, clears the marker, and then performs further updates. The emulator must reproduce those separate writes rather than replacing the routine with an atomic host save transaction.

### 10.12 BMA150

The owner needs a physical signal path and its digital state:

```text
Input trajectory and calibration
Conversion phase
Filter history and startup fill
Published axis/temperature data
Per-axis shadow and freshness state
Interrupt qualification history
Working configuration and nonvolatile image
Sleep/wake/self-test/programming state
Serial-interface state
```

The Bosch datasheet specifies moving-average digital filtering and describes wake-up initially operating at maximum bandwidth until enough samples exist for the selected filter. Replacing this with immediate register writes or one generic wake delay loses relevant behavior. ([Digi-Key][10])

The exact filter representation must preserve its recurrence, rounding, and startup history. A plausible filter with the same nominal cutoff is not sufficient.

Noise generation, when included, must be indexed by physical sample progression—not by register reads or run-call count.

Unknown calibration-field effects must remain identified research questions, not silently writable bytes with fabricated semantics.

### 10.13 NT7508

Keep separate:

```text
Serial byte assembly
Pending command parameters
Controller RAM
Address and bitplane phase
Display controls
Scan/latch state
Clock and drive configuration
```

This separation is required by an actual `pw` sequence: its byte-sending helper deasserts chip select after a complete byte, while contrast command and parameter are sent through separate calls. The lifetime of a pending command parameter therefore cannot simply be equated with one selected serial window.

The core must implement the controller rather than a retail two-bank abstraction. `pw`’s bank convention remains ordinary firmware activity expressed through controller commands.

---

## 11. Serial acceleration without an atomic-byte shortcut

### 11.1 Use an edge-run interface

The serial domain should accept an exact run of clock edges bounded by the next possible outside influence.

Conceptually:

```text
Advance serial domain to boundary:
    determine how many edges precede the boundary;
    stop at any point where a driver rule or parser phase changes;
    apply the corresponding shift/sample evolution;
    process completed protocol boundaries;
    continue while the outside boundary is not reached.
```

The same operation handles one edge or many.

The optimization is algebraic shifting over a known signal segment, not calling `transfer_byte()` through a separate route.

### 11.2 Valid bounds

A run cannot cross an unprocessed:

```text
Chip-select change
Pin-function change
External signal change
Clock change
Receiver/transmitter control change
Device operation affecting its output
Parser transition that changes the next driven bits
Caller horizon
```

This makes partial-byte behavior unavoidable in the canonical representation while allowing ordinary stable transfers to avoid excessive global scheduling.

### 11.3 Device parsers own their transaction lifetimes

A reusable shift helper may accumulate bits.

It must not decide that deselection resets every device’s command state.

The LCD, EEPROM, and sensor have different relationships between:

```text
Byte assembly
Command assembly
Selected interval
Ongoing internal operation
```

Those differences belong in the respective owners.

### 11.4 High impedance is real state

An undriven line is not a transmitted byte of ones.

Resolve drivers and pulls explicitly on shared nets. Contention must not be arbitrarily converted into a convenient Boolean operation unless that matches the electrical model.

Do not identify a recipient by firmware intent. Multiple selected devices must interact through the actual wiring.

---

## 12. Lossless output descriptions

### 12.1 Do not make presentation cost a hardware requirement

An exact signal need not be stored as one record per edge.

For a stable timer waveform, retain:

```text
Source clock projection
Phase
Initial level
A bounded pattern of transitions
Repetition period
Start of validity
```

A configuration change ends that segment and starts another.

This is not sound-effect substitution. The description still determines every voltage transition.

SameBoy reports a performance improvement of up to 17% from lazy APU output generation. That is evidence that output work can be reduced independently of hardware omission; it is not a promised speedup for HachiStep. ([SameBoy][11])

### 12.2 Buzzer example

A stable waveform lasting a minute should not require the core to enqueue every audible transition merely because a frontend might later request PCM samples.

The core should expose its exact drive law. The audio adapter evaluates or integrates that law at the requested output sample rate.

If firmware changes a compare register halfway through a pulse, the old law ends at that exact instant. The replacement begins with the correct phase and level.

If both piezo terminals change simultaneously, resolve their differential voltage jointly. Do not emit a spurious intermediate pulse from host field-update order.

### 12.3 Output compression cannot erase feedback

A timer edge may also affect an interrupt, capture input, or another modeled electrical condition.

Those consequences still occur.

The scheduler may omit an edge only when all relevant effects remain represented correctly. Whether a frontend has audio enabled must not determine that.

### 12.4 No irrevocable predictions

A periodic description may describe an ongoing rule, but the frontend may only consume it through the core’s **committed-time watermark**.

Later firmware can replace the rule.

The core therefore publishes:

```text
Signal/control changes at established times
A returned horizon through which execution is committed
```

It does not publish speculative future history and later retract it.

### 12.5 Display output

The LCD owner should resolve scan and latch dependencies at relevant RAM/control changes, and expose lossless display-state changes.

Pixel conversion is requested work, not mandatory work on every serial byte.

Controller interpretation remains core-owned. A frontend should not need to reimplement the NT7508 command parser to display the result.

### 12.6 Output delivery

Use a synchronous borrowed sink at product-output boundaries.

The sink must not re-enter the machine. It may copy, consume, or discard output without changing hardware execution.

The core maintains no unbounded event history and performs no ordinary-run allocation.

Persistence events must identify the operation's actual committed data or status. “EEPROM changed; inspect it later” loses information when several commits occur before inspection.

### 12.7 Frontend integration

Reassess display/audio integration when building the first frontend. At that
point, decide which pixel conversion, audio sampling and filtering, and retained
presentation history belong in helpers that can be reused across frontends.
Choose the physical appearance and sound model from those concrete requirements.
This is a future design task, not a requirement to build presentation machinery
before the core or to commit now to calibrated LCD/piezo simulation.

Controller behavior, timing, and effects that feed back into the emulated machine
remain core responsibilities. The lossless output contracts above preserve the
information needed for presentation; they do not require each frontend to expand
raw electrical signals itself or settle the eventual pixel/audio buffer API.
Requested helpers can perform that conversion without compulsory rendering work
in ordinary execution.

Established emulator examples are compared in
[emulator presentation boundaries](research/emulator-presentation.md).

---

## 13. Public API and save states

### 13.1 Public surface

Build the Rust core and Rust embedding API first. Native C-compatible and
JavaScript/Wasm bindings remain future adapters; their implementation and ABI
design do not gate core development.

Before release, change the Rust API and internal organization when doing so makes
the design better, updating in-repository callers together. Do not add compatibility
layers merely to preserve starter interfaces. When interfaces disagree, first
consider changing the interfaces themselves; compatibility requires a concrete
external need.

The ordinary API should remain small:

```text
Construct from firmware, persistent state, and unit conditions.
Apply timestamped physical inputs.
Run to an exclusive horizon.
Inspect derived display/output state.
Read committed nonvolatile state.
Perform explicit power lifecycle operations.
Capture and restore causal state.
```

Debug and hardware-fixture interfaces are separate advanced surfaces.

There is no public blank object requiring callers to manipulate hidden registers to make it runnable.

### 13.2 Inputs

Product inputs describe physical conditions:

```text
Button positions
Device-frame specific force
Temperature where relevant
Supply conditions
Incident infrared signal
```

No `add_steps`, `receive_packet_into_ram`, or `set_retail_clock_counter` operation belongs in the hardware core.

Motion input includes a defined coordinate frame and interpolation rule. A piecewise-linear trajectory is suitable; a constant segment is simply a zero-slope segment.

The frontend owns conversion from its host sensor recording into that trajectory. The core must not pretend missing high-frequency information was measured.

### 13.3 Simultaneous inputs

Changes to independent properties at one timestamp form a batch.

Contradictory assignments to the same property at the same instant are rejected rather than resolved by list order.

Input validation should be amortized. Do not rescan the full input history on every short run call.

### 13.4 Save-state contents

Use the ordinary emulator distinction: EEPROM saves preserve firmware progress;
save states preserve a running machine. A snapshot is the in-memory form of a
save state, not a separate persistence feature. Other nonvolatile hardware state
remains modeled alongside EEPROM contents.

A save state retains all future-determining state:

```text
CPU registers, instruction/exception progress, and actual fetched material
RAM, internal flash, and nonvolatile device contents
Device configuration and physical conditions
Partial accesses and exception progress
Clock phases and power transitions
Interrupt-admission state
Peripheral latches and histories
Serial shifters and parser state
Sampled analog values and conversion progress
Nonvolatile operations in progress
Deterministic physical-model state
```

It excludes:

```text
Decode caches
Host pointers and callbacks
File paths and sockets
Rendered images as hardware authority
Profiler state
Rebuildable scheduler caches
```

An owner’s true completion target or remaining clock obligation is causal state. The calendar entry derived from it is not an independent authority.

### 13.5 Coupled machines

Two instances must never exchange signals by advancing one into the future and injecting its output into the other’s past.

A coordinator may stop the same executor at outward-signal boundaries and process coupled same-time effects conservatively.

This requires no second CPU engine. It is another use of the same suspension and boundary contract.

### 13.6 Native save-state design

Design HachiStep's own exact save states now. Cross-emulator interoperability is
deferred, while saved-state semantics should support eventual convergence.
Do not introduce save-state versioning, migrations, or compatibility machinery
before the project approaches 1.0.

Capture at the stopped API boundary, retaining any CPU or peripheral operation
in progress. Saving must neither advance emulated time nor execute guest reads,
flush hardware operations to completion, or change subsequent behavior.

Define the saved-state contract in hardware terms before choosing its encoding.
Specify what retained values mean, their units and timing reference, which
effects have completed, and what remains in progress. CPU state must cover
fetched material, latched operands/results, access progress, remaining clock
obligations, and interrupt/exception progress wherever these determine future
behavior. A generated executor continuation number alone does not describe that
contract. Define enough to reconstruct the future effects; a transistor-level
description of undocumented internals is not required.

The running executor may retain its compact continuation identifiers. Map them
to and from documented operation progress during save/load, generating those
mappings where useful. This adds no ordinary-run bookkeeping and does not require
a second executor, a universal intermediate language, or a duplicated live state.
Use the same types directly where their meanings already match the contract.

Borsh remains a leading encoding candidate. Its language-independent byte rules
do not supply the meaning of the saved fields; readers must agree on that meaning
separately. Evaluate the codec against the documented state contract, including
exact restoration and practical capture/load costs. Large-array support is an
integration benefit, not sufficient justification by itself.

Encode owned state with explicit field widths and byte order. Share field
definitions between encoding and decoding, using ordinary Rust types and
build-time generation where that removes repetitive bookkeeping. Derive codecs
only where the types express the saved-state contract. Rust object layout,
incidental enum numbering, host pointers, and disposable caches must not
accidentally define an interchange format.

Decode and validate a complete candidate before replacing the live machine.
A failed load leaves the current session intact. Restore authoritative state,
then reconstruct derived scheduling and presentation without advancing time or
performing guest accesses. Full restoration includes the captured EEPROM and
other nonvolatile contents.

The core exposes capture, encoding, decoding, restoration, and persistent images.
Frontends own filenames, save slots, compression, file replacement, backups, and
when restored EEPROM is written to the ordinary save file. Completed capture
buffers can be processed outside emulation; serialization adds no work to normal
execution when no state is being captured.

Verify resumed behavior with identical input suffixes after captures during
partial instructions, serial transfers, timer activity, and nonvolatile
operations. Compare subsequent observable effects, not just encoded byte round
trips or private field layouts. Use the same capture machinery for future rewind
or debugging features if needed.

Native restoration must preserve exact subsequent hardware behavior. A future
interchange contract must explicitly choose whether it offers exact restoration
or only a usable session with some timing/state details reconstructed on a
best-effort basis. Reading the same bytes does not establish equivalent machine
behavior. BESS demonstrates a deliberate best-effort exchange alongside detailed
native state; that precedent does not select a second format or fallback path for
HachiStep. Decide any such tradeoff when interoperability work resumes.

The source-backed design rationale is in
[emulator saves and independent hardware tests](research/emulator-state-and-testing.md).

---

## 14. Independent hardware conformance project

### 14.1 Separate the specification evidence from the emulator

Create a separately versioned **Pokéwalker hardware test suite** that does not depend on `hs-core`.

Its contents should be useful to another emulator or a physical test runner.

The division is:

| Project               | Owns                                                                                                 |
| --------------------- | ---------------------------------------------------------------------------------------------------- |
| Hardware test suite   | Diagnostic programs, signal fixtures, expected observations, applicability, and measurement records. |
| HachiStep             | The production hardware implementation.                                                              |
| HachiStep adapter     | Loading/running cases and exporting observations.                                                    |
| HachiStep-local tests | Embedding contracts, save/restore behavior, resource guarantees, and concrete regressions.             |
| `pw`                  | Reconstructed firmware, workload understanding, and reduction context.                               |

Mooneye is a useful model: it separates hardware-oriented acceptance tests, emulator-oriented cases, and manual observations, and records model applicability. A separate repository is not unusual overhead; it is an established way to prevent the implementation from owning its own hardware truth. ([GitHub][12])

Keep the hardware suite separate. Specify cases as guest instructions, physical
inputs, and expected observable results so a different implementation can run
them unchanged. Local tests should protect meaningful contracts and regressions;
private struct layouts, helper-call sequences, and duplicated implementation
logic are not behavioral contracts. Generate large input sets where useful, but
derive expected behavior from documentation, hardware observations, or justified
inference rather than HachiStep's current output. The supporting examples from
Mooneye, SameSuite, mGBA, and Dolphin are recorded in the
[research note](research/emulator-state-and-testing.md#hardware-test-suites).

### 14.2 Keep the harness small

The shared guest harness needs only:

```text
Startup and linker support
Target definitions
Result storage
Failure identification
Completion reporting
Optional post-measurement output
```

It should not contain a high-level library for every peripheral being tested.

A timer test should be readable as register accesses and timing operations, not an interaction with an elaborate timer-testing object hierarchy.

### 14.3 Repository organization

A practical structure is:

```text
pokewalker-tests/
  common/
    startup/
    linker/
    target.inc
    result.inc

  cpu/
    arithmetic/
    addressing/
    flags/
    exceptions/
    fetch/
    timing/

  mcu/
    clocks/
    reset/
    interrupts/
    gpio/
    timers/
    rtc/
    watchdog/
    serial/
    analog/
    flash/

  devices/
    bma150/
    m95512/
    nt7508/
    infrared/

  board/
    shared-serial/
    wake/
    sound/
    power/

  research/
  observations/
  tools/
```

An ordinary test should be one source file plus an expected-data file only when necessary.

Large generated corpora and raw captures can be distributed as versioned data releases. Do not create one source file per random vector.

### 14.4 Three interfaces

| Interface              | What it establishes                                              |
| ---------------------- | ---------------------------------------------------------------- |
| Guest diagnostic image | Whole-machine behavior through actual CPU and register accesses. |
| Device signal fixture  | A component’s response to controlled electrical inputs.          |
| CPU state/vector case  | Instruction behavior under an explicitly defined CPU/bus setup.  |

All emulator adapters must invoke production owners and the production executor.

A CPU test bus is not another CPU implementation. It is a test environment, and its claims must be restricted accordingly.

### 14.5 Expected results must have a basis

Each test records:

```text
Stable identifier
Target hardware
Behavior being distinguished
Required setup
Expected observations
Basis: documentation, observation, firmware evidence, or reasoned inference
Known limitations
```

An unresolved expected result is not a passing test.

Use distinct outcomes:

```text
Pass
Fail
Unknown expectation
Not applicable
Runner/setup failure
```

Do not report one misleading “accuracy percentage” that mixes these categories.

---

## 15. Constructing tests that reveal mechanisms

### 15.1 The harness can change what is being measured

A test may execute successfully on hardware while measuring the wrong mechanism.

Gekkio’s test-ROM guidance documents this problem directly, including how using `HALT` changed interrupt timing and how CPU-level observations can obscure finer hardware edges. It recommends both focused tests and combination tables, not merely accumulating more isolated pass/fail programs. ([Gekkio][13])

For HachiStep:

* Do not use `SLEEP` as an allegedly neutral way to wait while measuring ordinary interrupt admission.
* Save CCR before comparison or reporting code changes it.
* Do not rely on an unverified timer to certify another timer’s absolute timing.
* Do not use a debugger configuration that changes the target resources without recording that fact.
* Perform reporting after the measurement window wherever possible.

### 15.2 Respect the actual measurement boundary

The H8’s internal flash and bus are not exposed like the external bus of an 8088.

SingleStepTests’ 8088 corpus uses hardware-generated state and bus observations, with explicit limitations including untested interrupt and wait-state behavior. The useful lesson is its evidence structure—not an assumption that the same internal observations are directly available on a Pokéwalker. ([GitHub][14])

For the Pokéwalker, distinguish:

```text
Directly observed external pin behavior
Guest-observed register values
Documented internal timing
Inferred internal sequencing
```

Do not label an emulator-generated internal bus trace a hardware capture.

### 15.3 Result reporting

Use a small linker-reserved RAM result structure.

It should contain a signature, case identifier, running/completed state, first failing subcase, and compact observed values.

Write completion last.

The host adapter inspects this memory without guest read side effects. A hardware reporting stage retrieves the same results after measurement.

The result area must be relocatable for tests that need to exercise that RAM region. Tests must fit the target’s small RAM without a guest-side serialization framework.

### 15.4 Exhaustive and directed coverage

Use exhaustive enumeration where the space is tractable:

```text
All first-word decode classifications
Byte arithmetic operands and relevant incoming flags
Register alias combinations
Small control-field combinations
```

Use directed grids for timing:

```text
Before, at, and after the relevant edge
Enabled and disabled
Previously clear and previously set
Relevant power and clock states
Relevant access width and access kind
```

Use sequences for history-dependent behavior:

```text
Read-before-clear qualification
Interrupt deferral
Sensor shadowing
Buffer transfers
Clock-divider phase
Nonvolatile command qualification
```

Large arithmetic corpora do not replace short sequences that expose timing and history.

### 15.5 Combination coverage follows real connections

Build interaction tests from the machine’s actual dependency graph:

```text
Clock control × timer
Timer event × status access
GPIO mux × serial activity
Serial completion × interrupt admission
RTC update × stable-read loop
Supply transition × programming operation
Buzzer drive × connected analog conditions
```

Do not blindly generate the Cartesian product of every register with every other register.

### 15.6 Independent expectations

A generator can create inputs. It must not automatically obtain all expected results from the candidate emulator.

A shared encoding declaration can validate decoder consistency. It cannot independently prove its own correctness.

Maintain separate calibration and validation cases for analog or timing models. A filter fitted to a waveform must be validated against other waveforms, not certified by replaying only its fitting data.

---

## 16. Initial conformance cases

These cases directly exercise architectural decisions that otherwise tend to be hidden by ordinary retail operation.

| Test                                 | Controlled experiment                                                                    | What it distinguishes                                                          |
| ------------------------------------ | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| **Exception stack order**            | Observe the two stack destinations and interrupt/reset the sequence between accesses.    | Correct final frame versus correct physical write order.                       |
| **Discarded fetch behavior**         | Arrange controlled instruction-fetch observations around exception entry.                | Actual prefetch activity versus invented internal delay.                       |
| **RTC update window**                | Read each time register around its update phase; perform the two-snapshot sequence.      | Atomic host time versus actual busy/update behavior.                           |
| **RTC deferred dispatch**            | Run a workload that defers firmware time handling while RTC interrupts continue.         | Hardware execution versus high-level elapsed-time compensation.                |
| **Watchdog reset identity**          | Trigger watchdog reset without removing external-device power.                           | Correct reset domain and firmware-visible reset cause.                         |
| **Timer clear race**                 | Sweep a flag-clear access across the event that sets the flag.                           | Local conflict semantics versus arbitrary scheduler order.                     |
| **Clock gate phase**                 | Disable and re-enable a downstream module at several parent-clock phases.                | Retained divider phase versus restart-at-write behavior.                       |
| **Serial holding versus shifting**   | Queue data and sample status around holding-register and shift-register boundaries.      | Distinct hardware states versus one transfer-complete flag.                    |
| **LCD parameter across deselection** | Send command and parameter in separate complete-byte selected intervals.                 | Byte assembly lifetime versus command-parser lifetime.                         |
| **BMA wake filter fill**             | Apply a controlled input while waking at different bandwidth settings.                   | Actual filter startup versus a fixed delay followed by steady-state filtering. |
| **Interrupted staged commit**        | Interrupt an isolated storage workload at several accepted/committed write boundaries.   | Real persistent sequence versus atomic host transaction.                       |
| **Waveform reconfiguration**         | Change timer configuration midway through a pulse and compare reconstructed transitions. | Lossless signal representation versus frequency-only audio approximation.      |

The first group can use documented expectations where the manuals settle the result. A measurement-dependent case stays in the research category until its expectation is established.

Tests involving flash wear, persistent changes, or physical power interruption must be explicitly classified. They should not silently run as part of an ordinary non-destructive hardware test batch.

---

## 17. Verification of the optimizations themselves

Hardware conformance and representation correctness are different obligations.

### 17.1 Partition invariance

Run each scenario as:

```text
One long call
Fixed short calls
Randomly partitioned calls
Calls ending immediately before, at, and after important effects
```

Compare canonical state and the same observable history.

An instruction, serial transfer, sensor update, or programming operation must not change behavior because the host chose a different call size.

Replay representative cases on native and browser targets with identical
initialization and inputs. Compare hardware observations and reconstructed
signals across hosts, allowing equivalent output segmentation as described below.

### 17.2 Clock arithmetic

Compare bulk clock advancement with an independent rational calculation.

Test:

```text
Fractional remainder
Large intervals
Source changes
Gating
Stabilization
Snapshot/restore
Epoch rebasing
```

A timestamp representation that slowly changes oscillator frequency through rounding is a fidelity defect.

### 17.3 Compressed outputs

Expand signal descriptions in the test runner and compare all transitions over the test interval.

Do not compare only final level or nominal frequency.

Exercise initial transients, configuration changes, simultaneous terminal changes, clock changes, and restoration midway through a period.

Different output segmentation is acceptable only when the reconstructed signal is identical.

### 17.4 Cache invariance

Repeat scenarios with empty, warm, and forcibly invalidated decode metadata.

Include RAM execution, extension-word changes, cross-boundary instructions, flash programming, and mutation before versus after actual fetching.

### 17.5 Canonical state

Lazy owners may hold different internal representations of the same current state.

Comparison must normalize or project them consistently without performing guest-visible reads.

Raw struct equality is not a sufficient definition once some counters or outputs are represented analytically.

### 17.6 No production reference engine

Small independent recurrences and mathematical oracles belong in tests.

A complete second “slow correct Pokéwalker” does not belong in production. It would duplicate the hardest semantics and recreate the dual-engine maintenance problem.

---

## 18. Failure localization and the role of `pw`

`pw` should be used to understand real workloads and reduce discrepancies—not to replace firmware execution.

A productive debugging sequence is:

```text
Find the first divergent hardware observation.
Identify the firmware operation that exposed it.
Preserve the relevant surrounding clock, interrupt, and peripheral state.
Reduce the case into an independent hardware question.
Fix the owning mechanism.
Keep both the reduced case and the original workload.
```

The first wrong screenshot is usually too late. Look for the first wrong register read, interrupt admission, pin transition, or persistent mutation.

The `pw` RTC code is particularly useful because it exposes a deliberate stable-read algorithm and deferred software handling. The startup code exposes reset-cause handling and multi-operation persistence. Those are better integration workloads than arbitrary NOP loops.

Diagnostics should capture compact records near the divergence. Symbol lookup, source annotation, disassembly formatting, and trace presentation belong in host tooling.

Begin with timed execution, side-effect-free inspection, and optional compact
traces. The starter already provides these capabilities. Do not build a debugger
framework in advance of a concrete need. Add stepping or stop conditions when an
actual debugging task justifies them, through the same executor and suspension
contract. Such additions must preserve peripheral progress and guest timing.

Disabled diagnostics must not allocate records, format strings, update atomic counters, or route every arithmetic effect through an observer.

---

## 19. Performance specification

Native and browser execution are target uses. Optimize portable algorithms, data representations, and eliminated work. Do not introduce ARM64- or WebAssembly-specific tricks or alternative execution paths; the compiler may perform its normal target-specific lowering.

Interactive embedding shapes the public interface. Interactive use and batch firmware execution are both first-class performance workloads through the same core. Neither receives a reduced hardware model.

### 19.1 The intended cost structure

Host work should scale primarily with:

$$
\text{CPU semantic work}
+\text{necessary component evolution}
+\text{cross-component interactions}
+\text{input changes}
+\text{output-description changes}.
$$

It should not automatically scale with:

$$
\text{every clock edge of every component}
+\text{every potential output sample}.
$$

Some sensor histories or interaction-heavy firmware genuinely require substantial work. The goal is to remove redundant representation work, not to pretend those workloads are free.

### 19.2 Hot-path requirements

| Path                 | Required property                                          |
| -------------------- | ---------------------------------------------------------- |
| Register instruction | Direct arithmetic and a cheap timing budget update.        |
| Ordinary RAM access  | Direct width-aware access without virtual device dispatch. |
| Register access      | Synchronize only affected owners.                          |
| Inactive peripheral  | No per-instruction work.                                   |
| Timer evolution      | Exact arithmetic where applicable.                         |
| Serial evolution     | Canonical edge-run processing, no allocation.              |
| Stable buzzer output | Compact signal law, no mandatory per-edge global event.    |
| LCD mutation         | Controller update, not full host framebuffer conversion.   |
| Sleep                | Advance to the next relevant consequence.                  |
| Trace disabled       | No tracing work in the semantic path.                      |

### 19.3 Allocation and memory

Construction may allocate fixed backing storage. Ordinary execution must not allocate.

Do not allocate:

```text
Instruction objects
Effect vectors
Per-register objects
Per-edge event nodes
Per-byte serial transactions
CPU snapshots for rollback
```

Track production code, generated code, immutable tables, causal state, and disposable metadata separately.

Generated code is still code size. A small source generator producing an enormous executable is not inherently succinct.

### 19.4 Benchmark matrix

Anchor optimization decisions in realistic Pokéwalker execution: retail workloads and plausible custom firmware. Synthetic cases are useful for isolating costs and exercising mechanisms, but a large synthetic speedup does not establish a meaningful improvement for the project. Verify the benefit in the relevant realistic workload.

The performance corpus must include:

| Workload                                  | Reason                                                       |
| ----------------------------------------- | ------------------------------------------------------------ |
| Cold construction and first execution     | Exposes hidden predecode/setup cost.                         |
| Register and branch-heavy custom firmware | Measures CPU execution overhead.                             |
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

### 19.5 Default-build comparison

Pin the source revision, compiler, build settings, target, input timeline, unit parameters, and output consumer.

Use repeated paired runs with varied ordering and report uncertainty.

Report at least:

```text
Wall and CPU time
Memory footprint
Allocation count
Compiled text/table size
Short-call latency distribution
Relevant active-execution throughput
Energy where measured
```

Do not claim a benchmark measured energy when it measured only elapsed time.

Do not compare one emulator dropping sound or link behavior with another preserving it without identifying that difference.

At the same time, an approximate competitor being faster is still useful evidence. It should motivate optimization, not be hidden by redefining the comparison.

### 19.6 Acceptance of an optimization

Every optimization must state:

```text
Which repeated work it removes
Which state or timing invariant makes that removal valid
Which independent tests exercise the affected behavior
Which workloads improve
Which workloads regress
What code-size and memory cost it adds
```

The production build contains the selected implementation, not a permanent menu of experimental engines.

There is no accuracy toggle to make a benchmark win.

---

## 20. Build and completion strategy

Implementation should proceed in complete vertical mechanisms, not by constructing a framework and postponing hardware behavior.

| Stage                                 | Concrete deliverable                                                                                              |
| ------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| **Clock and execution foundation**    | One executor, exact boundaries, clock-phase arithmetic, direct bus access, and partition tests.                   |
| **CPU completeness**                  | Target encoding coverage, instruction semantics, fetch behavior, exception sequencing, and independent CPU cases. |
| **Clock/reset/interrupt integration** | Correct reset domains, source relationships, admission, and wake behavior.                                        |
| **Serial domain**                     | One electrical path shared by hardware serial and GPIO, with device fixtures.                                     |
| **External components**               | Complete command/state models, including sensor history and nonvolatile state.                                    |
| **Remaining MCU mechanisms**          | Timers, RTC, watchdog, event counter, analog, and flash contracts.                                                |
| **Lossless output integration**       | Signal descriptions, display changes, persistence events, and reconstruction tests.                               |
| **Hardware closure**                  | Focused experiments resolving consequential unknowns.                                                             |
| **Competitive optimization**          | Default-build measurements across the complete workload matrix.                                                   |

Performance measurement starts with the first executable slice. Accuracy testing starts with the first implemented mechanism.

A component is not complete because it accepts all register writes. It is complete when its accesses, hidden state, timing, reset/power behavior, interactions, and applicable corner cases are accounted for.

A test suite is not independent merely because it lives in another repository. Its expectations must also have an independent basis.

A compact implementation is not one with the fewest source lines. It is one where each retained state variable and abstraction has a clear causal purpose, and each hardware effect has one implementation.

---

## Final architectural contract

**One statically compiled timed executor. One authority for each hardware fact. Clock-local progress between interactions. Exact compressed evolution where the mechanism permits it. Lossless output descriptions instead of compulsory presentation work. Independent hardware tests rather than a self-certifying runtime.**

The strongest optimization opportunity is not to execute the wrong machine faster. It is to stop making the host repeatedly rediscover, reschedule, and reformat behavior that the correct machine model already determines.

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
