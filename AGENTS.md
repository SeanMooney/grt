# AGENTS.md — grt

## First Actions

Do these before anything else:

1. Read `ai/session.md` — current focus, open questions, handoff notes
2. Read `manifest.toon` — index of every document in the project; use it to find what you need
3. Read `ai/rust-conventions.md` before writing any code

## Project Identity

grt is a Rust CLI/TUI tool for managing Git and Gerrit workflows. It provides local SQLite caching, async I/O, and both interactive and scriptable interfaces.

**Current phase:** Design — documentation scaffolding complete, no application code yet.

**Core crates:** clap (CLI), ratatui (TUI), tokio (async runtime), rusqlite (SQLite), reqwest (HTTP), serde (serialization).

For the full tech stack and architecture, read `ai/context.md`.

## Build & Development

```bash
cargo build              # Build
cargo test               # Test
cargo clippy             # Lint
cargo fmt -- --check     # Check formatting
```

## Knowledge Base Navigation

All project knowledge lives in documentation files. Use `manifest.toon` to find them — don't scan the filesystem.

- **`manifest.toon`** — Document index with path, type, status, topics, and summary for every file
- **`ai/`** — Agent working memory:
  - `context.md` — Stable project context and tech stack
  - `session.md` — Mutable session state (read at start, update at end)
  - `rust-conventions.md` — Coding standards (single source of truth)
  - `patterns.md` — Pattern library (grows over time)
- **`docs/design/`** — Design docs describing how grt will work (prescriptive)
  - `README.md` — Document map, traceability matrix, reading paths
  - `decisions/` — Architecture Decision Records
- **`docs/design/ref-specs/`** — Analysis of prior art used as research input
  - Read the relevant ref-spec before writing a design doc
  - The traceability matrix in `docs/design/README.md` maps ref-specs to design docs

### Principles

- **Single source of truth** — no file duplicates another
- **No sync obligations** — cross-references are links, not duplicated content
- **Incremental population** — stubs exist for all planned docs; status tracked in `manifest.toon`

## Session Protocol

At the end of your session:

1. Update `ai/session.md` with what you did and what comes next
2. Record new patterns in `ai/patterns.md`

## Reference Projects

The `ref/` directory (gitignored) contains source code from three prior-art projects studied during design. See `docs/design/ref-specs/README.md` for details.
