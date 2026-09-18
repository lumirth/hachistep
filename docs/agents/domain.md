# Domain docs

Use a single context for this workspace:

- `CONTEXT.md` at the repository root holds the domain glossary.
- `docs/DESIGN.md` holds the current design contract.
- `docs/adr/` holds rationale for consequential architecture decisions when needed.

## Before exploring the codebase

Read root `CONTEXT.md` and any ADRs relevant to the area being explored.

Create glossary entries and ADRs only when terms or consequential tradeoffs are resolved.
An absent ADR directory is not a documentation defect.

## Use the glossary's vocabulary

Use the terms defined in `CONTEXT.md` when naming domain concepts in issues, proposals,
hypotheses, code, and tests. Respect any explicitly avoided synonyms.

Add missing terms when they clarify a real hardware or product distinction. Keep
implementation details in the design or code, not the glossary.

## Flag ADR conflicts

If a proposal contradicts an existing ADR, identify the ADR and explain why the decision
should be reopened before overriding it. Update the affected design contract with the
decision so an ADR and `docs/DESIGN.md` do not become competing authorities.
