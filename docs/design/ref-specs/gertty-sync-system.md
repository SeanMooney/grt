# Gertty Sync System

**Source project:** gertty
**Source files:** `gertty/sync.py`, `gertty/db.py` (sync-related tables)
**Status:** Stub
**Informs:** `sync-engine.md`, `gerrit-client.md`

## Overview

<!-- TODO: How gertty synchronizes local SQLite with Gerrit server -->

## Priority Queue

<!-- TODO: Task priority levels and scheduling -->

## Task Types

<!-- TODO: Document 30+ sync task types (SyncChangeTask, SyncProjectTask, etc.) -->

### Inbound Tasks
<!-- Server → local: fetching changes, comments, approvals -->

### Outbound Tasks
<!-- Local → server: submitting reviews, votes -->

### Maintenance Tasks
<!-- Pruning, re-sync, conflict detection -->

## Offline Handling

<!-- TODO: How gertty queues operations when server is unreachable -->

## Conflict Resolution

<!-- TODO: How gertty handles concurrent modifications -->

## Rate Limiting

<!-- TODO: How gertty avoids overwhelming the Gerrit server -->

## grt Divergences

<!-- TODO: Where grt's sync will differ:
- Tokio async vs Python threading
- Channel-based task queue vs Python Queue
- Potential for incremental/streaming sync
-->
