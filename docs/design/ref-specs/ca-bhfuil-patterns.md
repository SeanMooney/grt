# ca-bhfuil Architectural Patterns

**Source project:** ca-bhfuil
**Source files:** `src/ca_bhfuil/`, `docs/`, `ai/memory/`
**Status:** Draft
**Informs:** `architecture.md`

## Overview

ca-bhfuil is a Python CLI tool for tracking commits across stable branches in git repositories. Its architecture is organized around several key patterns: a manager class hierarchy for coordinating business logic and database persistence, an async-first infrastructure built on asyncio for concurrent repository operations, and a CLI layer using Typer that bridges synchronous command entry points into the async core.

The patterns most relevant to grt are:

1. **Manager hierarchy with factory and registry** -- ca-bhfuil uses `BaseManager` / `ManagerFactory` / `ManagerRegistry` to wire up managers with shared database sessions. The factory handles initialization, dependency injection, and lifecycle; the registry tracks live instances by key. This pattern solves the problem of coordinating multiple managers that share resources, but introduces complexity around ownership and mutable shared state.

2. **Async bridge for CLI commands** -- A thin decorator (`@async_command`) wraps every async CLI handler so Typer (which is synchronous) can invoke it through `asyncio.run()`. A `with_progress()` helper ties Rich progress spinners to arbitrary coroutines. This is a clean pattern for bolting async business logic onto a synchronous CLI framework.

3. **Operation result types** -- Every manager method returns a typed result object (`OperationResult`, `CommitSearchResult`, `RepositoryAnalysisResult`) carrying success/failure, duration, error messages, and domain-specific payload. This is a uniform error-handling pattern that avoids exceptions for expected failures.

4. **Concurrent sync with semaphore control** -- Repository synchronization uses `asyncio.Semaphore(3)` to cap concurrent operations, `asyncio.gather()` with `return_exceptions=True` for fan-out, and a thread pool executor for blocking pygit2 calls. This is a practical concurrency model for I/O-bound workloads with CPU-bound sub-operations.

5. **Global singleton pattern for services** -- Most infrastructure components (`ManagerFactory`, `AsyncConfigManager`, `AsyncRepositoryRegistry`, `AsyncRepositorySynchronizer`) expose a module-level `get_*()` function that lazily creates and caches a singleton instance. This simplifies wiring but creates hidden global state.

## Manager Pattern

ca-bhfuil structures its business logic around a three-part manager system: a `BaseManager` abstract base class, a `ManagerFactory` that creates and wires up concrete managers, and a `ManagerRegistry` that tracks live instances.

The central concrete implementation is `RepositoryManager`, which orchestrates operations between git repositories (via pygit2) and the SQLite database layer. Additional managers could follow the same pattern for future domains (e.g., analysis, search).

### Manager Lifecycle

**Creation.** Managers are created through `ManagerFactory`, which handles database initialization and dependency injection:

```python
class ManagerFactory:
    async def initialize(self) -> None:
        self._db_manager = sqlmodel_manager.SQLModelDatabaseManager(self._db_path)
        await self._db_manager.initialize()
        await self._registry.set_shared_database_manager(self._db_manager)
        self._initialized = True

    async def get_repository_manager(self, repository_path):
        await self.initialize()  # idempotent
        repo_manager = RepositoryManager(
            repository_path=repository_path,
            db_manager=self._db_manager,
        )
        manager_key = f"repository:{repository_path}"
        self._registry.register(manager_key, repo_manager)
        return repo_manager
```

The factory supports async context manager protocol (`__aenter__`/`__aexit__`) for scoped lifecycle management. It also exposes module-level convenience functions (`get_repository_manager()`) that go through a lazily-initialized global factory instance.

**Initialization.** The `initialize()` method is idempotent -- calling it multiple times is safe because it checks `self._initialized`. This lets callers be defensive without worrying about double-init.

**Teardown.** Cleanup follows an ownership chain: `ManagerFactory.close()` calls `ManagerRegistry.close_all()`, which iterates all registered managers and calls their `.close()` methods. Each `BaseManager.close()` clears its references and closes the database manager only if it owns it (tracked by `self._manager_owned`):

```python
async def close(self) -> None:
    self._db_session = None
    self._db_repository = None
    if self._manager_owned and self._db_manager:
        await self._db_manager.close()
        self._db_manager = None
```

### Resource Ownership

`BaseManager` uses a dual-ownership model for database resources. Each manager receives an optional `db_session` and `db_manager` at construction. If either is `None`, the manager creates its own and marks it as owned:

```python
def __init__(self, db_session=None, db_manager=None):
    self._db_session = db_session
    self._db_manager = db_manager
    self._session_owned = db_session is None
    self._manager_owned = db_manager is None
```

This allows managers to share a database connection (when created by the factory with a shared `db_manager`) or operate independently (when created standalone). The `_database_session()` context manager either reuses the injected session or creates a new one through the database manager.

`RepositoryManager` additionally owns a `git_repository.Repository` wrapper around pygit2, created at construction from the repository path. This git handle is not shared between managers.

Resources owned by each manager:

- **BaseManager**: Database session (optional), database manager (optional), database repository facade
- **RepositoryManager**: git repository wrapper (always owned), plus inherited BaseManager resources
- **ManagerRegistry**: Shared database session, shared database manager, dictionary of all registered managers
- **ManagerFactory**: The ManagerRegistry, the shared database manager

### Inter-Manager Communication

Managers do not communicate directly with each other. Instead, coordination happens through two mechanisms:

1. **Shared database session** -- The `ManagerRegistry.set_shared_session()` method propagates a database session to all registered managers by reaching into their private `_db_session` attribute. This is a runtime mutation pattern that ensures all managers see the same database state within a request:

```python
async def set_shared_session(self, session):
    self._db_session = session
    for manager in self._managers.values():
        if hasattr(manager, "_db_session"):
            manager._db_session = session
            manager._db_repository = None  # Force recreation
```

2. **Registry lookup** -- The `ManagerRegistry.get()` method allows retrieval of any registered manager by key (e.g., `"repository:/path/to/repo"`). Return type is `typing.Any`, with type safety restored through caller-side type hints. In practice, cross-manager lookups do not appear to be used in the current codebase -- managers are independent.

There is also an implicit coordination path through the database itself: `RepositoryManager.sync_with_database()` writes commit data that other components (like `AsyncRepositoryRegistry`) can later read.

## Async Task Management

ca-bhfuil's async infrastructure is split across several modules, each handling a distinct concurrency concern.

**Core task management** (`async_tasks.py`): `AsyncTaskManager` provides a minimal background-task system. It wraps coroutines in `asyncio.create_task()`, assigns UUID-based task IDs, and tracks status through a `TaskStatus` enum (PENDING, RUNNING, COMPLETED, FAILED). Results and exceptions are stored in dictionaries keyed by task ID:

```python
class AsyncTaskManager:
    def create_task(self, coro) -> str:
        task_id = str(uuid.uuid4())
        task = asyncio.create_task(self._run_task(task_id, coro))
        self._tasks[task_id] = task
        self._status[task_id] = TaskStatus.RUNNING
        return task_id

    async def _run_task(self, task_id, coro):
        try:
            result = await coro
            self._results[task_id] = result
            self._status[task_id] = TaskStatus.COMPLETED
        except Exception as e:
            self._results[task_id] = e
            self._status[task_id] = TaskStatus.FAILED
```

This is a fire-and-forget pattern -- there is no cancellation API, no task cleanup, and no mechanism to await completion. Results accumulate in memory indefinitely.

**Concurrency control** (`async_sync.py`, `async_repository.py`): Concurrent operations are governed by `asyncio.Semaphore` to limit parallelism. `AsyncRepositorySynchronizer` uses `Semaphore(3)` to cap concurrent repository syncs. `AsyncRepositoryManager` uses a configurable semaphore (default 5) for general concurrent operations:

```python
class AsyncRepositorySynchronizer:
    def __init__(self):
        self._sync_semaphore = asyncio.Semaphore(3)

    async def sync_repository(self, repo_name):
        async with self._sync_semaphore:
            # ... sync logic ...

    async def sync_repositories_concurrently(self, repo_names):
        tasks = [self.sync_repository(name) for name in repo_names]
        results = await asyncio.gather(*tasks, return_exceptions=True)
```

Fan-out uses `asyncio.gather(*tasks, return_exceptions=True)`, which collects all results including exceptions rather than failing fast. Results are post-processed to separate successes from failures.

**Thread pool for blocking operations** (`async_git.py`, referenced throughout): Since pygit2 is synchronous, ca-bhfuil runs git operations in a thread pool executor via `AsyncGitManager.run_in_executor()`. This keeps the event loop responsive while pygit2 does blocking I/O.

**Progress tracking** (`async_progress.py`): `AsyncProgressTracker` uses an `asyncio.Queue` to bridge progress updates from synchronous contexts (thread pool workers) to async consumers. A background task consumes the queue and invokes an async callback:

```python
class AsyncProgressTracker:
    def __init__(self, progress_callback):
        self._queue = asyncio.Queue()
        self._consumer_task = asyncio.create_task(self._consume_progress())

    def report_progress(self, progress_obj):
        loop = asyncio.get_running_loop()
        asyncio.run_coroutine_threadsafe(self._queue.put(progress_obj), loop)
```

This is a sound pattern for cross-thread progress reporting. The `report_progress()` method is callable from synchronous code running in the executor, and the queue consumer runs on the event loop.

**Operation monitoring** (`async_monitor.py`): `AsyncOperationMonitor` provides a `@timed` decorator that tracks call counts, success/failure rates, and total duration for async functions. Statistics are stored in a plain dictionary.

**Error handling with retry** (`async_errors.py`): `AsyncErrorHandler` implements exponential backoff with jitter. It accepts a coroutine factory (not a coroutine) so that each retry creates a fresh coroutine:

```python
async def retry(self, coro_factory, retry_on):
    backoff = self.initial_backoff
    for attempt in range(self.attempts):
        try:
            return await coro_factory()
        except retry_on:
            if attempt == self.attempts - 1:
                raise
            sleep_time = backoff + random.uniform(0, backoff * 0.1)
            await asyncio.sleep(sleep_time)
            backoff = min(self.max_backoff, backoff * 2)
```

**Async config** (`async_config.py`): `AsyncConfigManager` mirrors the synchronous `ConfigManager` but uses `aiofiles` for non-blocking file I/O. It includes an `asyncio.Lock` for cache protection, though the cache itself (`_config_cache`) does not appear to be populated in the current implementation.

## CLI/Library Separation

ca-bhfuil separates its CLI presentation layer from its business logic through a layered architecture: Typer commands in `cli/main.py` handle argument parsing and output formatting, while the core logic lives in manager classes and async service modules.

### Entry Points

The CLI is built with Typer, organized into a root app and subcommand groups:

```python
app = typer.Typer(name="ca-bhfuil", no_args_is_help=True)
config_app = typer.Typer(name="config", no_args_is_help=True)
repo_app = typer.Typer(name="repo", no_args_is_help=True)
db_app = typer.Typer(name="db", no_args_is_help=True)

app.add_typer(config_app, name="config")
app.add_typer(repo_app, name="repo")
app.add_typer(db_app, name="db")
```

This produces a command tree: `ca-bhfuil config {init,validate,status,show}`, `ca-bhfuil repo {add,list,update,remove,sync}`, `ca-bhfuil search`, `ca-bhfuil status`.

Every async command uses the `@async_command` decorator from `cli/async_bridge.py` to bridge from Typer's synchronous world into asyncio:

```python
@repo_app.command("sync")
@async_command
async def repo_sync(name: str | None = typer.Argument(None), ...):
    synchronizer = async_sync.AsyncRepositorySynchronizer()
    sync_result = await with_progress(
        synchronizer.sync_repository(repo.name),
        f"Syncing {repo.name}...",
    )
```

The bridge is minimal -- `async_command` wraps the function with `asyncio.run()`, and `with_progress()` displays a Rich spinner while an operation runs. The bridge module is 68 lines total.

CLI commands follow a consistent pattern:
1. Parse arguments (Typer handles this)
2. Get configuration via `async_config.get_async_config_manager()`
3. Create or obtain the relevant service (synchronizer, manager factory, etc.)
4. Run the operation via `with_progress()` for user feedback
5. Format and display results using Rich tables, panels, and syntax highlighting
6. Handle errors with consistent `[red]` error messages and `typer.Exit(1)`

**Shell completion** (`completion.py`): Tab completion is implemented as standalone functions (`complete_repository_name`, `complete_repo_path`, `complete_format`) that Typer's `autocompletion` parameter references. Notably, `complete_repository_name` uses the synchronous `ConfigManager` rather than the async version, since shell completion must return immediately. A manually-written bash completion script supplements Typer's built-in completion.

### Library API

The core library exposes functionality through several async service classes, each with a global singleton accessor:

| Service | Accessor function | Responsibility |
|---|---|---|
| `ManagerFactory` | `get_manager_factory()` | Creates and wires managers |
| `AsyncConfigManager` | `get_async_config_manager()` | Configuration loading/saving |
| `AsyncRepositoryRegistry` | `get_async_repository_registry()` | Repository state tracking |
| `AsyncRepositorySynchronizer` | `get_async_repository_synchronizer()` | Repository sync operations |

CLI commands interact with these services through the accessor functions. There is also a synchronous `ConfigManager` used in non-async contexts (shell completion, tests).

The `operations.py` module in the CLI package contains async functions (`config_init_async`, `config_validate_async`) that duplicate logic from `main.py`. These appear to be an attempt at extracting testable async operations separate from the Typer command boilerplate, but the approach was not carried through consistently -- most commands still have their logic inline in the command function.

### Shared Types

Several Pydantic models serve as the shared contract between CLI and core layers:

- **`config.RepositoryConfig`** -- Defines a repository's name, source, branches, sync strategy, storage settings, and auth reference. Used by both CLI commands and core services.
- **`config.GlobalConfig`** -- Top-level configuration with version, repos list, and settings dict.
- **`results.OperationResult`** -- Base result type with `success`, `duration`, `error`, and `result` fields. Extended by domain-specific result types.
- **`commit.CommitInfo`** -- Pydantic model representing a git commit with SHA, author, dates, message, and diff stats. Includes methods like `matches_pattern()` and `calculate_impact_score()`.
- **`progress.TaskStatus`** -- Enum tracking background task states (PENDING, RUNNING, COMPLETED, FAILED).
- **`progress.OperationProgress`** -- Progress data model with total, completed, and status fields.

The CLI layer formats these types into Rich tables and panels. The core layer produces them from git and database operations. The types flow upward -- core never depends on CLI types.

## Documentation System

ca-bhfuil uses a two-tier documentation system: authoritative design documents in `docs/contributor/design/` and mutable AI session context in `ai/memory/`.

### What Worked

**Structured design docs.** The `docs/contributor/design/` directory contains focused, single-topic documents: `architecture-overview.md`, `cli-design-patterns.md`, `concurrency.md`, `repository-management.md`, `technology-decisions.md`, etc. Each has a clear purpose, audience, and cross-references to related docs. The README provides a reading guide and relationship map. This organization makes it easy to find authoritative information on a specific topic.

**AI memory for session state.** The `ai/memory/` directory gives AI agents a place to record decisions, progress, and handoff notes without polluting the design docs. Files like `current-focus.md`, `bootstrap-tasks.md`, and `patterns.md` provide mutable scratch space that persists between sessions. The separation between "authoritative docs" and "working memory" is a useful distinction.

**Session handoff protocol.** The CLAUDE.md file defines a detailed handoff protocol: read memory files at start, update them at end, use a structured template for progress/blockers/decisions. This ensures continuity across AI development sessions.

**Decision documentation.** `ai/memory/architecture-decisions.md` provides an ADR-like format with context, alternatives, rationale, impact, and reversibility sections. This captures not just what was decided but why.

### What Didn't (and how grt's system fixes it)

**Content duplication between tiers.** The `ai/memory/` files duplicate substantial content from `docs/contributor/design/`. For example, `ai/memory/project-context.md` restates the architecture overview, and `ai/memory/ai-style-guide.md` duplicates `docs/contributor/code-style.md` with explicit sync obligations ("when code-style.md is updated, ai-style-guide.md MUST be updated immediately"). This creates a maintenance burden and a risk of divergence. The CLAUDE.md file itself is over 400 lines of instructions, many of which reiterate content from design docs.

**No document index.** There is no machine-readable manifest of all documents, their types, statuses, or topics. CLAUDE.md lists key files in prose, and the design README provides a table, but neither supports programmatic lookup. An agent must scan directories or read CLAUDE.md top-to-bottom to find the right document.

**Sync obligations.** Multiple places in CLAUDE.md mandate keeping files in sync: "When `docs/contributor/code-style.md` changes: IMMEDIATELY update `ai/memory/ai-style-guide.md`." These obligations are easy to forget and impossible to enforce automatically.

**Excessive CLAUDE.md length.** At 400+ lines, the CLAUDE.md file tries to serve as entry point, development guide, quality checklist, troubleshooting manual, and session protocol simultaneously. Much of the content is duplicated from other files or is procedural boilerplate that does not vary between sessions.

**grt's fixes:**

- **`manifest.toon`** provides a machine-readable document index with path, type, status, topics, and summary for every file. Agents can look up documents programmatically rather than scanning.
- **No sync obligations** -- grt's AGENTS.md explicitly states that cross-references are links, not duplicated content. No file duplicates another.
- **Single source of truth** -- coding standards live in `ai/rust-conventions.md` alone, not duplicated in a second "AI-optimized" version.
- **Concise entry point** -- grt's AGENTS.md is under 50 lines, directing agents to the relevant files rather than repeating their content.
- **Incremental population** -- stubs exist for all planned docs with status tracking in the manifest, making it visible what exists and what remains to be written.

## grt Divergences

### Rust ownership model vs Python manager classes

ca-bhfuil's `BaseManager` / `ManagerFactory` / `ManagerRegistry` pattern exists to solve a problem that Python's runtime does not solve natively: tracking which component owns which resource, ensuring shared resources are initialized before use, and cleaning up in the right order. The `_session_owned` / `_manager_owned` booleans, the `hasattr()` checks in `set_shared_session()`, and the `close_all()` teardown loop are all manual ownership tracking.

Rust's ownership and borrowing system handles this at compile time. grt will not need a `ManagerRegistry` to track live instances or boolean flags to track ownership. Instead, a single `App` struct will own all major subsystems (database pool, git handles, config), and methods will borrow from it with explicit lifetimes. Resource cleanup is handled by `Drop` implementations rather than explicit `close()` calls.

The factory pattern (`ManagerFactory`) maps loosely to a builder pattern or `App::new()` constructor that initializes all subsystems in the right order. But instead of lazy initialization with `_initialized` guards, grt will initialize eagerly at startup and pass references downward.

### Tokio vs asyncio

ca-bhfuil's async architecture wraps synchronous pygit2 calls in `asyncio.to_thread()` / executor pools, uses `asyncio.Semaphore` for concurrency limits, and bridges sync-to-async at the CLI boundary with `asyncio.run()`.

grt will use tokio, which provides:

- **`tokio::task::spawn_blocking()`** instead of `asyncio.to_thread()` for blocking gix calls
- **`tokio::sync::Semaphore`** instead of `asyncio.Semaphore` for concurrency control
- **`tokio::sync::mpsc`** channels instead of `asyncio.Queue` for progress reporting
- **Structured concurrency** via `tokio::task::JoinSet` instead of `asyncio.gather()`, with the ability to cancel all tasks when the set is dropped

The `AsyncProgressTracker`'s pattern of using a queue to bridge synchronous and asynchronous contexts translates directly to tokio's `mpsc` channel, which is designed for this exact scenario. The `AsyncErrorHandler`'s retry-with-backoff pattern maps to a similar utility built on `tokio::time::sleep()`.

One key difference: tokio tasks are `'static` by default (they cannot borrow from the spawning scope), which is more restrictive than asyncio's coroutines. grt will use `tokio-scoped` for tasks that need to borrow from parent data, or pass owned data into tasks.

### App struct as central orchestrator vs multiple managers

ca-bhfuil distributes orchestration across multiple service classes: `AsyncRepositoryManager`, `AsyncRepositorySynchronizer`, `AsyncRepositoryRegistry`, `ManagerFactory`, plus the managers themselves. Each has a global singleton accessor. CLI commands create these services ad hoc and wire them together:

```python
config_manager = await async_config.get_async_config_manager()
synchronizer = async_sync.AsyncRepositorySynchronizer()
sync_result = await synchronizer.sync_repository(repo.name)
```

grt will consolidate this into a single `App` struct that owns the database pool, configuration, and git subsystem. CLI commands receive `&App` and call methods on it. This eliminates the need for global singletons, service locator patterns, and the `get_*()` accessor functions that create hidden shared state.

The ca-bhfuil pattern of having `AsyncRepositoryRegistry`, `AsyncRepositoryManager`, and `RepositoryManager` as separate classes with overlapping responsibilities (all three can look up and operate on repositories) reflects organic growth. grt's `App` struct will provide a unified interface, with internal modules handling git, database, and config concerns.

### grt's doc system fixes ca-bhfuil's duplication and sync problems

As detailed in the Documentation System section, ca-bhfuil's dual-tier documentation (design docs + AI memory) creates sync obligations and duplication. grt's system avoids this with three mechanisms:

1. **`manifest.toon`** -- A single index file that catalogs every document with its path, type, status, topics, and one-line summary. Agents use it for lookup instead of scanning the filesystem or parsing prose instructions.

2. **No-duplication rule** -- Each fact lives in exactly one file. Cross-references use links, not duplicated text. There is no "AI-optimized" copy of any document.

3. **Concise AGENTS.md** -- Under 50 lines. It tells agents what to read, not what the files contain. The files speak for themselves.

These fixes are structural, not just aspirational -- the manifest format enforces discoverability, and the no-duplication rule eliminates the class of sync-obligation bugs that ca-bhfuil's CLAUDE.md spends significant effort trying to manage.
