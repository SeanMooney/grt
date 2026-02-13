# ca-bhfuil Architectural Patterns

**Source project:** ca-bhfuil
**Source files:** `src/ca_bhfuil/`, `docs/`, `ai/memory/`
**Status:** Stub
**Informs:** `architecture.md`

## Overview

<!-- TODO: ca-bhfuil's architecture and the patterns worth adapting -->

## Manager Pattern

<!-- TODO: How ca-bhfuil uses manager classes to coordinate operations -->

### Manager Lifecycle
### Resource Ownership
### Inter-Manager Communication

## Async Task Management

<!-- TODO: asyncio patterns, task coordination, cancellation -->

## CLI/Library Separation

<!-- TODO: How ca-bhfuil separates CLI entry points from business logic -->

### Entry Points
### Library API
### Shared Types

## Documentation System

<!-- TODO: ca-bhfuil's docs/ + ai/memory/ system — what worked and what didn't -->

### What Worked
### What Didn't (and how grt's system fixes it)

## grt Divergences

<!-- TODO: Where grt will differ:
- Rust ownership model vs Python manager classes
- Tokio vs asyncio
- App struct as central orchestrator vs multiple managers
- grt's doc system fixes ca-bhfuil's duplication and sync problems
-->
