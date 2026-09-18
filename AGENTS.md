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

## Documentation

Keep durable contracts, usage instructions, primary-source references, and nonobvious
hardware rationale in the repository. Update their existing home when a decision
changes. Keep progress, handoffs, completed audit reports, and individual change
validation summaries in the task, commits, or issues. Git preserves superseded
documents; remove them instead of maintaining a parallel work log.

## Agent skills

### Issue tracker

Use GitHub Issues through `gh`. Before ticket work, read `docs/agents/issue-tracker.md`.

### Triage labels

Use the five default triage labels. Before triaging or changing labels, read
`docs/agents/triage-labels.md`.

### Domain docs

Use a single context: root `CONTEXT.md` and `docs/adr/`. Before exploring the codebase,
read `docs/agents/domain.md` for the consumer rules.
