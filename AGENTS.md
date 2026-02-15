# AGENTS.md — grt

## First Actions

Do these before anything else:

1. Read `ai/session.toon` — current focus, open questions, handoff notes
2. Read `manifest.toon` — index of every document in the project; use it to find what you need
3. Read `docs/design/adopted/rust-conventions.md` before writing any code
4. Read `docs/design/adopted/repo-layout.md` to understand the repo structure

## Project Identity

grt is a Rust CLI/TUI tool for managing Git and Gerrit workflows. It provides local SQLite caching, async I/O, and both interactive and scriptable interfaces.

**Current phase:** Design — documentation scaffolding complete, Cargo workspace bootstrapped with build infrastructure.

**Core crates:** clap (CLI), ratatui (TUI), tokio (async runtime), sqlx (SQLite), reqwest (HTTP), serde (serialization).

For the full tech stack and architecture, read `ai/context.toon`.

## Build & Development

```bash
cargo build              # Build
cargo test               # Test
cargo clippy             # Lint
cargo fmt -- --check     # Check formatting
just test                # Run tests via just
just lint                # fmt + clippy + deny
nix develop              # Enter dev shell with all tools
```

## Commit Requirements

### DCO Sign-off

Every commit must include a `Signed-off-by` trailer. When creating commits, always
use the `-s` flag or include the trailer manually. This is enforced by a commit-msg
hook.

### SPDX Headers

Every `.rs` file must start with:

```rust
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors
```

Do not modify the header on `crates/grt/resources/commit-msg` (vendored Gerrit file).

## Knowledge Base Navigation

All project knowledge lives in documentation files. Use `manifest.toon` to find them — don't scan the filesystem.

- **`manifest.toon`** — Document index with path, type, status, topics, and summary for every file
- **`docs/design/adopted/repo-layout.md`** — Authoritative directory structure; read this to understand where everything lives
- **`ai/`** — Agent working memory (TOON format where beneficial; read `ai/toon-spec.md` for format details):
  - `context.toon` — Stable project context and tech stack
  - `session.toon` — Mutable session state (read at start, update at end)
- **`docs/design/`** — Design docs describing how grt will work (prescriptive)
  - `README.md` — Document map, traceability matrix, reading paths
  - `draft/` — Design docs in progress
  - `adopted/` — Implemented and authoritative design docs
    - `rust-conventions.md` — Coding standards (single source of truth)
    - `patterns.md` — Pattern library (grows over time)
  - `decisions/` — Architecture Decision Records
- **`docs/design/ref-specs/`** — Analysis of prior art used as research input
  - Read the relevant ref-spec before writing a design doc
  - The traceability matrix in `docs/design/README.md` maps ref-specs to design docs

### Principles

- **Single source of truth** — no file duplicates another
- **No sync obligations** — cross-references are links, not duplicated content
- **Incremental population** — stubs exist for all planned docs; status tracked in `manifest.toon`

## Claude Code Skill

grt ships a Claude Code plugin in `.claude-plugin/` that teaches the `/grt` skill. When you add or change grt CLI capabilities, update the skill to match:

1. **New command or flag** — update `references/grt-commands.md` and add a workflow section in `SKILL.md` if the command introduces a new user-facing workflow.
2. **Changed JSON output** — update `references/comment-json-schema.md` to reflect the new schema. The source of truth is the struct definitions in `crates/grt/src/comments.rs`.
3. **New exit code or error** — update the error handling table in `SKILL.md` and the exit codes table in `references/grt-commands.md`.
4. **New write API operation** (e.g., submit, vote) — remove the corresponding "cannot do this" note from the safety rules in `SKILL.md` and add the workflow.

The skill must stay in sync with `docs/user/cli-reference.md` — the CLI reference is the source of truth, the skill is a derived, agent-optimized view.

## Session Protocol

At the end of your session:

1. Update `ai/session.toon` with what you did and what comes next
2. Record new patterns in `docs/design/adopted/patterns.md`

## Reference Projects

The `ref/` directory (gitignored) contains source code from three prior-art projects studied during design. See `docs/design/ref-specs/README.md` for details.
