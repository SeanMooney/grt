# grt Architecture

**Related ref-specs:** `ref-specs/ca-bhfuil-patterns.md`
**Status:** Stub

## Overview

<!-- TODO: High-level system design for grt -->

## Module Boundaries

<!-- TODO: What each module owns and its public API surface -->

### main.rs — Entry Point
### app.rs — Orchestrator
### db.rs — Data Layer
### gerrit.rs — Gerrit Client
### git.rs — Git Operations
### notedb.rs — NoteDb Reader
### fuzzy.rs — Search
### tui.rs — Terminal UI

## Data Flow

<!-- TODO: How data moves through the system -->

### Inbound (Gerrit → local)
### Outbound (local → Gerrit)
### Query (local → display)

## Concurrency Model

<!-- TODO: Scoped vs unscoped tasks, channel topology -->

## Startup and Shutdown

<!-- TODO: Initialization order, graceful shutdown -->

## Error Propagation

<!-- TODO: How errors flow between modules, see also error-handling.md -->
