# Agent guidance

Treat HachiStep as a fresh implementation. Never consult past HachiStep implementations,
their documentation, or project memories. Ground decisions in this repository, agreed
requirements, hardware documentation, observations, and the `pw` decompilation.

Before hardware, architecture, interface, or performance work, read `docs/design.md`.
Revise a design choice when concrete consequences justify it. Keep source interpretations
and unresolved physical assumptions in `docs/hardware.md` or beside the mechanism.

Demonstrate behavior through Hachiware guest programs and complete firmware sessions.
Use `docs/testing.md`. Do not add unit, integration, tautological, or change-detector
tests. Add a bug regression check only for a genuine behavioral coverage gap.
Reuse completed checks unless a change, failure, or open concern warrants another run.

Keep docs minimal and useful to their reader. Preserve goals, usage contracts, primary
sources, and nonobvious rationale. Let code explain its own structure. Update the owner
of a topic, consolidate overlap, and fix affected references. Keep progress and individual
validation reports in the task, commits, PRs, or issues. Apply `unslop` and `clean-copy`;
use `writing-for-agents` when editing agent instructions.

Use GitHub Issues through `gh`. Read an issue before ticket work and inspect existing
labels before triage. Use a file with `--body-file` for multiline issue and PR prose.

At substantive milestones, report hardware fidelity, frontend usability, realistic
performance, and architecture results, with remaining gaps, in the task or PR.
