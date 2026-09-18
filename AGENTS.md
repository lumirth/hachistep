# Agent guidance

## Fresh implementation

Treat HachiStep as a fresh implementation. Never consult or reference past HachiStep
implementations, their documentation, or their project memories. Ground decisions in
this repository, requirements agreed here, hardware documentation, observed behavior,
and the `pw` firmware decompilation.

## Design baseline

Before hardware, architecture, interface, or performance work, read the relevant
sections of `docs/DESIGN.md`. It records the design contract and agreed clarifications.
Continue to question assumptions through concrete consequences; revise the design when
those consequences justify a different choice.

## Documentation and prose

Keep durable contracts, usage instructions, references to primary sources, and nonobvious
rationale in the repository. Keep progress, handoffs, completed audit reports, and
individual validation summaries in the task, commits, or issues.

Update the existing owner of a topic; consolidate overlap and remove superseded material
after preserving useful reasoning. Split topics when it improves navigation. Resolve
contradictions and affected references as part of the change, proportional to the work.

Write for the reader: state the point directly, explain what matters, and preserve precise
hardware terms and notation. Use lists when they help comparison or navigation. Comments
should explain behavior, constraints or nonobvious reasoning. Cut filler, stock conclusions,
rhetorical contrasts and invented labels. Use `writing-for-agents` for agent instructions,
not general documentation.

## Evidence and completion

Use `docs/TESTING.md` for validation and evidence limits. At substantive milestones,
explain the results and remaining gaps for the affected goals: hardware fidelity,
frontend usability, realistic performance and a compact, coherent architecture. Keep
this in the task or issue.

## Project references

- Before ticket work, read `docs/agents/issue-tracker.md`; use GitHub Issues through `gh`.
- Before triaging or changing labels, read `docs/agents/triage-labels.md`.
- Before exploring the codebase, read `docs/agents/domain.md` and root `CONTEXT.md`.
