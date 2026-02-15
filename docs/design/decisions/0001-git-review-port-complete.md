# ADR-0001: git-review port complete

**Status:** Accepted
**Date:** 2026-02-15
**Context area:** cli, git-review parity

## Context

grt was designed as a drop-in replacement for git-review. After implementing the core workflows (push, download, list, cherry-pick, compare, setup) and addressing low-impact gaps, the port is complete for common use cases.

## Options Considered

### Option A: Declare port complete
- Mark the git-review port as complete modulo future bug fixes
- Focus development on TUI, SQLite, and new features
- Pros: Clear milestone; avoids scope creep
- Cons: None significant

### Option B: Continue parity work
- Keep implementing every git-review edge case
- Pros: Maximum compatibility
- Cons: Diminishing returns; custom hooks explicitly not desired

## Decision

Consider the git-review port complete modulo future bug fixes. Custom pre/post-review hooks remain unsupported by design. The following gaps were addressed in Phase 17:

- `--use-pushurl` — wired via CliOverrides into config layer
- `--color` / `--no-color` — control `color.remote` for git push
- Compare-mode rebase — optionally rebase both patchsets onto target before diffing
- `--no-custom-script` — accepted for compatibility, no-op (we don't run custom scripts)

## Consequences

- Focus shifts to TUI, SQLite, and new features
- git-review parity maintenance is minimal unless bugs are reported
- Design docs (cli-design.md) updated to reflect current divergences
