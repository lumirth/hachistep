# Design goals

HachiStep should make Pokéwalker firmware development practical: accurate enough to
predict hardware behavior, fast enough for long sessions and broad experiments, and
small enough to understand and maintain.

## One hardware model

Retail and custom firmware use the same mechanisms. Firmware identity, known routines,
and familiar register sequences must not select shortcuts. Full fidelity is the
default. Optimizations must preserve physical access order, timing, side effects,
and the state that can affect later behavior.

Model the board as connected components. Keep one authority for each hardware effect.
Components may summarize evolution between interactions when the result preserves all
observable consequences. Internal execution techniques can change when they improve
speed or clarity without changing those consequences.

Read target manuals with amendments and errata. Use board observations and the matching
`pw` decompilation to resolve how components connect and how firmware uses them. When
sources leave a gap, choose a concrete mechanism with a stated basis. Keep the reasoning
beside that mechanism or in the [hardware notes](HARDWARE.md).

## Time and reproducibility

Emulated time belongs to the device. Host pacing, rendering cadence, and transport
belong to the application. Regrouping host calls must preserve every hardware effect
and its timestamp. Matching initial state and inputs must produce matching results on
native and WebAssembly hosts.

A save state must retain everything needed for the same future behavior, including
unfinished accesses, clock phases, and programming operations. Frontend queues and
presentation state remain the host's responsibility.

Physical inputs describe buttons, acceleration, optical levels, pins, and supply.
Firmware owns steps, menus, protocols, and save layouts. Host memory editing is an
explicit debugging operation, separate from physical input.

## Performance and maintainability

The performance goal is at least 10 times PocketWalker's speed on every Hachiware
workload while passing the workload's independent checks. Measure the default release
build and publish individual results. A weighted average cannot substitute for a slow
workload.

Optimize measured costs. Consider the cost of construction, ordinary run calls, memory,
and capture as well as long-run throughput. Reuse hardware semantics across execution
paths and keep qualifications for faster paths explicit.

Code size is a useful signal of disproportionate complexity. Keep additional code when
its correctness or performance benefit justifies maintaining it. Remove duplicated
ownership and obsolete paths when revising the design.

## Product and evidence

The core must be usable by a frontend without knowledge of its internal scheduler or
component synchronization. The CLI must support reproducible sessions and useful
outputs with ordinary files and commands.

Hachiware owns independently specified guest programs, firmware sessions, behavioral
checks and shared measurements. HachiStep owns emulation, its CLI and the adapter
that exposes it to those experiments. New coverage should answer a real behavioral
question.

An EEPROM save contains persistent bytes. A save state captures a running session;
its in-memory representation is a snapshot. Use those distinctions consistently.

Revise these choices when concrete consequences justify it. Record implementation
progress and individual validation results in commits, PRs, or issues.
