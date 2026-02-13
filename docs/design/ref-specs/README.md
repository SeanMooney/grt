# Reference Project Specifications

These documents capture how the Python reference projects work. They are **descriptive** (observational), not **prescriptive** — grt makes its own design decisions in the design docs in `../draft/` and `../adopted/`.

## Reference Projects

### gertty (4 specs)

Console UI for Gerrit written in Python (urwid + SQLAlchemy + SQLite). The most feature-complete reference and primary source for data model, sync, search, and UI patterns.

| Spec | What It Covers | Status |
|------|---------------|--------|
| `gertty-data-model.md` | SQLAlchemy schema: 18+ tables, relationships, indices, migrations | Complete |
| `gertty-sync-system.md` | Priority queue, 30+ task types, offline handling, conflict resolution | Complete |
| `gertty-search-language.md` | Tokenizer, parser grammar, query semantics, operator mapping | Complete |
| `gertty-config-and-ui.md` | YAML config, views, navigation model, keybindings, themes | Complete |

**Source code:** `ref/gertty/`

### git-review (2 specs)

CLI tool for submitting Git branches to Gerrit. Primary reference for the push workflow and Gerrit API protocols.

| Spec | What It Covers | Status |
|------|---------------|--------|
| `git-review-workflow.md` | Push workflow, Change-Id hooks, rebase handling, config layers | Complete |
| `git-review-gerrit-api.md` | SSH + HTTP protocols, auth methods, API version handling | Complete |

**Source code:** `ref/git-review/`

### ca-bhfuil (1 spec)

Python git analysis tool. Reference for async patterns, manager architecture, and CLI separation.

| Spec | What It Covers | Status |
|------|---------------|--------|
| `ca-bhfuil-patterns.md` | Manager pattern, async task management, CLI/library separation | Complete |

**Source code:** `ref/ca-bhfuil/` (has its own CLAUDE.md and extensive docs)

## How to Use These Specs

1. **Read before designing.** Before writing a grt design doc, read the relevant ref-specs to understand prior art.
2. **Note divergences.** Each ref-spec has a "grt Divergences" section noting where grt is expected to differ.
3. **Don't copy patterns blindly.** The Python projects have different constraints (GIL, dynamic typing, different concurrency models). Adapt, don't translate.
4. **Trace to design docs.** See the traceability matrix in `docs/design/README.md` for which ref-specs feed which design docs.
