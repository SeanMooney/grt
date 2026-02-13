# Repository Layout

**Status:** Adopted

Authoritative directory structure for grt. All other documents reference this file rather than
duplicating tree diagrams.

## Overview

grt uses a Cargo workspace with a single binary crate. Build infrastructure files live at the
repository root. Documentation is organized into design docs (draft/adopted lifecycle),
reference specifications, and AI agent memory files.

## Directory Tree

```
grt/
├── CLAUDE.md                       # AI agent entry point (AGENTS.md)
├── manifest.toon                   # Document index for agentic RAG
├── Cargo.toml                      # Workspace root
├── Cargo.lock                      # Committed — reproducible builds
├── rust-toolchain.toml             # Pinned Rust version + components
├── deny.toml                       # cargo-deny: licenses, advisories, bans
├── release.toml                    # cargo-release configuration
├── flake.nix                       # Nix flake: dev shell + reproducible build
├── flake.lock                      # Committed — pins all Nix inputs
├── justfile                        # Task runner (wraps cargo/nix commands)
├── CHANGELOG.md                    # Keep-a-Changelog format
├── .gitignore
│
├── .cargo/
│   └── config.toml                 # Cross-compilation linker overrides
│
├── .config/
│   └── nextest.toml                # cargo-nextest profiles (default + CI)
│
├── crates/
│   └── grt/                        # Primary binary crate
│       ├── Cargo.toml
│       └── src/
│           └── main.rs             # Entry point, CLI parsing
│
├── docs/
│   └── design/
│       ├── README.md               # Navigation hub, traceability matrix
│       ├── draft/                   # Design docs in progress
│       │   ├── architecture.md
│       │   ├── build-and-release.md
│       │   ├── tech-stack.md
│       │   ├── data-model.md
│       │   ├── gerrit-client.md
│       │   ├── cli-design.md
│       │   ├── tui-design.md
│       │   ├── search-engine.md
│       │   ├── config-system.md
│       │   ├── git-operations.md
│       │   ├── sync-engine.md
│       │   └── error-handling.md
│       ├── adopted/                 # Implemented and authoritative
│       │   ├── repo-layout.md       # This file
│       │   ├── rust-conventions.md
│       │   └── patterns.md
│       ├── ref-specs/               # Prior-art analysis (descriptive)
│       │   ├── README.md
│       │   ├── gertty-data-model.md
│       │   ├── gertty-sync-system.md
│       │   ├── gertty-search-language.md
│       │   ├── gertty-config-and-ui.md
│       │   ├── git-review-workflow.md
│       │   ├── git-review-gerrit-api.md
│       │   └── ca-bhfuil-patterns.md
│       └── decisions/               # Architecture Decision Records
│           └── README.md
│
├── ai/                              # AI agent working memory
│   ├── context.toon                 # Stable project context
│   ├── session.toon                 # Mutable session state
│   └── toon-spec.md                 # TOON format specification
│
└── ref/                             # Reference project source (gitignored)
```

### Planned directories (not yet created)

These directories will appear as implementation progresses:

- `ansible/playbooks/` — Zuul CI playbooks (see `draft/build-and-release.md`)
- `zuul.d/` — Zuul pipeline configuration
- `tests/` — Integration tests

## Workspace and Crate Organization

The workspace uses a `crates/` directory with `resolver = "2"`. Currently one crate:

| Crate | Type | Purpose |
|-------|------|---------|
| `grt` | Binary | CLI entry point and application |

Additional library crates will be extracted as the codebase grows beyond a single-crate
structure. The workspace `Cargo.toml` declares shared metadata (version, edition, license)
inherited by member crates.

## Source Module Layout

The `grt` binary crate will contain these modules, each with a single responsibility:

```
crates/grt/src/
├── main.rs      Entry point, CLI parsing, logging setup
├── app.rs       App struct, orchestration, shared state
├── db.rs        SQLite schema, queries, migrations
├── gerrit.rs    Gerrit REST client, auth, response parsing
├── git.rs       git2 operations, cherry-pick, status
├── notedb.rs    NoteDb ref parsing (Gerrit metadata in git)
├── fuzzy.rs     nucleo-matcher integration, search API
└── tui.rs       ratatui event loop, views, rendering
```

See `draft/architecture.md` for detailed module descriptions, public APIs, and
dependency relationships.

## Build and Release Files

| File | Purpose |
|------|---------|
| `Cargo.toml` | Workspace root with shared package metadata |
| `Cargo.lock` | Pinned dependency versions (always committed) |
| `rust-toolchain.toml` | Pinned Rust version, components, and targets |
| `deny.toml` | License allow/deny lists, advisory checks, source restrictions |
| `release.toml` | cargo-release: shared versioning, tag pattern, pre-release hook |
| `flake.nix` | Nix flake: dev shell, reproducible build, CI checks |
| `flake.lock` | Pinned Nix inputs (always committed) |
| `justfile` | Developer task runner (build, test, lint, deny, fmt) |
| `.cargo/config.toml` | musl rustflags, macOS cross-linker |
| `.config/nextest.toml` | Test runner profiles (default + CI with JUnit output) |
| `CHANGELOG.md` | Keep-a-Changelog format, updated before each release |

See `draft/build-and-release.md` for rationale on each tool choice and CI architecture.

## Documentation Structure

### Design doc lifecycle

Design docs move through two stages:

1. **`draft/`** — Work in progress. May be incomplete or contain open questions.
2. **`adopted/`** — Implemented and authoritative. The content reflects actual practice.

A doc moves from draft to adopted when its design is implemented and validated. This is a
`git mv`, preserving history.

### Reference specifications (`ref-specs/`)

Descriptive analysis of prior-art Python projects (gertty, git-review, ca-bhfuil). Read
before writing a design doc. The traceability matrix in `docs/design/README.md` maps
ref-specs to the design docs they inform.

### Architecture Decision Records (`decisions/`)

Immutable records of key design decisions. Created as decisions emerge during design and
implementation.

## AI Memory Files

The `ai/` directory contains files consumed exclusively by AI agents:

| File | Format | Purpose |
|------|--------|---------|
| `context.toon` | TOON | Stable project context: identity, tech stack, phase |
| `session.toon` | TOON | Mutable session state: what was done, what's next |
| `toon-spec.md` | Markdown | TOON format specification for reference |

TOON format is used where it provides better token efficiency for structured data. See
`ai/toon-spec.md` for format details.

Coding conventions and patterns live in `docs/design/adopted/` since they serve both AI
and human readers.

## Conventions

- **Cargo.lock committed** — required for reproducible builds of a binary application
- **flake.lock committed** — pins all Nix inputs to exact content-addressed revisions
- **Neither lock file is updated automatically** — both are updated deliberately and reviewed
