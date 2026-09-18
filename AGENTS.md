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

As you add or revise documentation, consider where readers will look for it. Prefer
updating existing material; split distinct topics when that improves navigation,
consolidate overlap, and remove superseded material after preserving useful reasoning.
Resolve contradictions and update affected references as part of the change. Keep this
upkeep proportional to the work.

Apply the same prose standards to documentation and code comments. State the main point
directly, use familiar words and active voice, and keep paragraphs focused. Use lists
when they make the content easier to scan or compare. Comments should explain behavior,
constraints, or nonobvious reasoning. Cut filler, stock conclusions, unprompted contrasts,
and invented or needlessly hyphenated labels. Keep technical caveats specific and useful.

## Agent skills

### Issue tracker

Use GitHub Issues through `gh`. Before ticket work, read `docs/agents/issue-tracker.md`.

### Triage labels

Use the five default triage labels. Before triaging or changing labels, read
`docs/agents/triage-labels.md`.

### Domain docs

Use a single context: root `CONTEXT.md` and `docs/adr/`. Before exploring the codebase,
read `docs/agents/domain.md` for the consumer rules.
