# Agent guidance

## Fresh implementation

Treat HachiStep as a fresh take based on this starter. Never consult or reference
past HachiStep implementations, their documentation, or their project memories.
Ground decisions in this repository, requirements agreed here, hardware
documentation, observed behavior, and the `pw` firmware decompilation.

## Design baseline

Before hardware, architecture, interface, or performance work, read the relevant
sections of `docs/DESIGN.md`. It records the approved starting design and agreed
clarifications. Continue to question assumptions through concrete consequences;
revise the design when those consequences justify a different choice.

## Agent skills

### Issue tracker

Use GitHub Issues through `gh`. Before ticket work, read
`docs/agents/issue-tracker.md`.

### Triage labels

Use the five default triage labels. Before triaging or changing labels, read
`docs/agents/triage-labels.md`.

### Domain docs

Use a single context: root `CONTEXT.md` and `docs/adr/`. Before exploring the
codebase, read `docs/agents/domain.md` for the consumer rules.
