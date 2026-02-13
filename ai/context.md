# Project Context: grt

**Status:** Design phase — no application code yet

## What grt Is

grt is a Rust CLI/TUI tool for managing Git and Gerrit workflows. It combines and ports functionality from three Python projects into a single, modern tool:

- **gertty** — Console UI for Gerrit with local SQLite caching, offline support, and a query language
- **git-review** — CLI for submitting Git branches to Gerrit (push workflow, hooks, config)
- **ca-bhfuil** — Git analysis tool demonstrating async manager patterns and CLI separation

grt is not a 1:1 port of any of these. It takes the best ideas from each and redesigns them for Rust idioms, unified architecture, and modern async patterns.

## Tech Stack

| Layer | Crate | Role |
|-------|-------|------|
| Runtime | tokio | Async task scheduling, channels, timers |
| Concurrency | tokio-scoped | Scoped tasks that borrow from parent |
| CLI | clap (derive) | Subcommands, type-safe args |
| TUI | ratatui + crossterm | Terminal UI, cross-platform input |
| Database | sqlx + SQLite | Async embedded SQL, compile-time checking |
| Git | git2 | libgit2 bindings, NoteDb access |
| HTTP | reqwest | Gerrit REST API client |
| Search | nucleo-matcher | fzf-quality fuzzy matching |
| Logging | tracing | Structured logging with spans |
| Errors | anyhow | Ergonomic error propagation |

See `tech-stack.md` for full details on each choice.

## Repository Layout

```
grt/
├── AGENTS.md              # AI agent guide (CLAUDE.md symlinks here)
├── tech-stack.md           # Technology selections and architecture
├── manifest.toon           # Document manifest for agentic RAG
├── docs/design/            # Design documentation
│   ├── ref-specs/          # Reverse-engineered reference project specs
│   ├── decisions/          # Architecture Decision Records
│   └── *.md                # grt design documents
├── ai/                     # AI agent working memory
│   ├── context.md          # This file — stable project context
│   ├── session.md          # Mutable session state and handoffs
│   ├── rust-conventions.md # Coding standards
│   └── patterns.md         # Pattern library
├── ref/                    # Reference projects (gitignored)
└── src/                    # Rust source (not yet created)
```

## Reference Projects

Located in `ref/` (gitignored). These are the Python projects grt draws from:

- `ref/gertty/` — The primary reference for data model, sync system, search, and UI
- `ref/git-review/` — The primary reference for push workflow and Gerrit API interaction
- `ref/ca-bhfuil/` — The primary reference for async patterns and CLI architecture

## Current Phase

Documentation scaffolding — building the knowledge base that will guide implementation.
Next phase: populating ref-specs by analyzing the reference projects.
