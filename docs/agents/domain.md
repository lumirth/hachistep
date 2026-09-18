# Domain docs

## Layout

Use a single context for this workspace:

- `CONTEXT.md` at the repository root holds the domain glossary.
- `docs/adr/` holds architecture decision records.

## Before exploring the codebase

Read root `CONTEXT.md` and any ADRs relevant to the area being explored.

If these files do not exist, proceed silently. Do not flag their absence or suggest
creating them upfront. `/domain-modeling`, including when reached through
`/grill-with-docs` or `/improve-codebase-architecture`, creates them lazily when terms
or decisions are resolved.

## Use the glossary's vocabulary

Use the terms defined in `CONTEXT.md` when naming domain concepts in issues, proposals,
hypotheses, code, and tests. Respect any explicitly avoided synonyms.

If a needed concept is missing, reconsider whether it belongs in the project; record an
actual vocabulary gap for `/domain-modeling`.

## Flag ADR conflicts

If a proposal contradicts an existing ADR, identify the ADR and explain why the decision
should be reopened before overriding it.
