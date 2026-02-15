# Architecture Decision Records

## Index

| ADR | Title | Status | Date |
|-----|-------|--------|------|
| [0001](0001-git-review-port-complete.md) | git-review port complete | Accepted | 2026-02-15 |

Decisions will be recorded here as they arise during design and implementation.

## Template

New ADRs should follow this format. Create files named `NNNN-short-title.md`.

```markdown
# ADR-NNNN: Title

**Status:** Proposed | Accepted | Deprecated | Superseded by ADR-XXXX
**Date:** YYYY-MM-DD
**Context area:** (e.g., data-model, sync, search, config)

## Context

What is the issue that we're seeing that is motivating this decision?

## Options Considered

### Option A: Name
- Description
- Pros
- Cons

### Option B: Name
- Description
- Pros
- Cons

## Decision

What is the change that we're proposing and/or doing?

## Consequences

What becomes easier or more difficult because of this decision?
```

## Conventions

- Number ADRs sequentially: `0001`, `0002`, etc.
- Accepted ADRs are immutable — if a decision changes, create a new ADR that supersedes the old one
- Link ADRs to the design docs they affect
- Keep each ADR focused on a single decision
