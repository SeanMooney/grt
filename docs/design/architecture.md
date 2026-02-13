# grt Architecture

**Related ref-specs:** `ref-specs/ca-bhfuil-patterns.md`, `ref-specs/gertty-sync-system.md`, `ref-specs/gertty-data-model.md`, `ref-specs/gertty-config-and-ui.md`
**Status:** Draft

For technology selections and crate rationale, see [tech-stack.md](tech-stack.md). This document covers how those technologies compose into a working system.

## Overview

grt is an async-first, local-first, dual-interface tool for managing Git and Gerrit workflows. Its architecture is organized around a central `App` struct that owns all subsystems and mediates their interactions.

**Design philosophy:**

- **Async-first.** All I/O (HTTP, SQLite, git) is non-blocking at the call site. Blocking operations (git2, rusqlite) run on dedicated blocking threads via `tokio::task::spawn_blocking`. The main event loop never blocks.
- **Local-first.** The SQLite database is the single source of truth for the UI. The user never waits for a network round-trip to view data. Background sync keeps the local cache current, and pending local mutations are persisted in the database so they survive restarts.
- **Dual-interface.** The same `App` struct serves both TUI and CLI modes. The TUI wraps it in an event loop with rendering; the CLI calls methods directly and prints results. No business logic lives in either interface layer.
- **App as orchestrator.** Inspired by the ca-bhfuil analysis ([ref-specs/ca-bhfuil-patterns.md](ref-specs/ca-bhfuil-patterns.md)), grt consolidates ownership into a single `App` struct rather than distributing it across multiple manager classes. Rust's ownership model makes this natural — `App` owns the database pool, config, and gerrit client, and lends references to subsystems that need them. No global singletons, no service locators, no runtime ownership tracking.

## Module Boundaries

Each module has a single responsibility, a defined public API, and explicit dependencies. The module layout matches `src/` files one-to-one.

```
src/
├── main.rs      Entry point, CLI parsing, logging setup
├── app.rs       App struct, orchestration, shared state
├── db.rs        SQLite schema, queries, migrations
├── gerrit.rs    Gerrit REST client, auth, response parsing
├── git.rs       git2 operations, cherry-pick, status
├── notedb.rs    NoteDb ref parsing (Gerrit metadata in git)
├── fuzzy.rs     nucleo-matcher integration, search API
└── tui.rs       ratatui event loop, views, rendering
```

### main.rs — Entry Point

**Responsibility:** Parse CLI args (clap), initialize tracing, route to TUI or CLI command.

**Public API:** `fn main()` — the binary entry point.

**Dependencies:** `app`, `tui`, all CLI-reachable modules.

**Key behaviors:**
- Parses `Args` using clap's derive API
- Initializes tracing-subscriber with the configured log level and output format
- Constructs `App` by calling `App::new(config).await`
- Routes to either `tui::run(&app).await` or the appropriate CLI subcommand (e.g., `app.list_changes()`, `app.sync()`)
- CLI subcommands return `anyhow::Result<()>` — errors print to stderr and exit with code 1

### app.rs — Orchestrator

**Responsibility:** Own all subsystems, provide the business logic API.

**Public API:**
- `App::new(config: Config) -> Result<App>` — construct and initialize
- `app.list_changes(query) -> Result<Vec<Change>>` — query local DB
- `app.sync() -> Result<SyncReport>` — trigger sync cycle
- `app.search(query) -> Result<Vec<ScoredChange>>` — fuzzy search
- `app.cherry_pick(change_id, branch) -> Result<()>` — git operation
- `app.show_change(id) -> Result<ChangeDetail>` — full detail fetch

**Owns:** `Config`, `Database` (sqlx pool or rusqlite connection), `GerritClient`, `GitRepo`.

**Dependencies:** `db`, `gerrit`, `git`, `notedb`, `fuzzy`.

**Design note:** `App` is not `Clone`. The TUI borrows `&App` (via `tokio-scoped`), and CLI commands receive `&App`. Long-running background tasks that outlive a scope receive owned handles (e.g., `GerritClient` clone, database pool clone).

**Ref-spec influence:** ca-bhfuil's `ManagerFactory` + `ManagerRegistry` pattern is replaced by direct ownership in `App`. See [ref-specs/ca-bhfuil-patterns.md § grt Divergences](ref-specs/ca-bhfuil-patterns.md#app-struct-as-central-orchestrator-vs-multiple-managers).

### db.rs — Data Layer

**Responsibility:** SQLite schema definition, migrations, all database queries.

**Public API:**
- `Database::open(path) -> Result<Database>` — open and migrate
- `db.upsert_change(change) -> Result<()>`
- `db.get_change(id) -> Result<Option<Change>>`
- `db.query_changes(filter) -> Result<Vec<Change>>`
- `db.get_pending_operations() -> Result<Vec<PendingOp>>`
- `db.mark_operation_complete(op_id) -> Result<()>`
- Transaction wrappers for multi-step operations

**Owns:** SQLite connection pool (or single connection with `Mutex`).

**Dependencies:** None (leaf module).

**Schema:** Adapted from gertty's 18-table model ([ref-specs/gertty-data-model.md](ref-specs/gertty-data-model.md)) with simplifications:
- Pending operations consolidated into a `pending_operation` table instead of scattered `pending_*` booleans
- `ON DELETE CASCADE` foreign keys instead of ORM-managed cascades
- Status columns use CHECK constraints instead of free-form strings
- Schema version tracked via SQLite's `user_version` pragma, migrations embedded in binary

### gerrit.rs — Gerrit Client

**Responsibility:** HTTP communication with Gerrit REST API, authentication, response deserialization.

**Public API:**
- `GerritClient::new(config) -> Result<GerritClient>`
- `client.get_change(id) -> Result<GerritChange>`
- `client.query_changes(query) -> Result<Vec<GerritChange>>`
- `client.submit_review(change_id, review) -> Result<()>`
- `client.get_version() -> Result<String>`
- All methods are `async`

**Owns:** `reqwest::Client` (with connection pooling), server URL, auth credentials.

**Dependencies:** None (leaf module — uses only reqwest and serde).

**Design note:** REST-only, no SSH. Strips Gerrit's XSSI prefix dynamically. Implements retry with exponential backoff for transient errors. See [ref-specs/git-review-gerrit-api.md § grt Divergences](ref-specs/git-review-gerrit-api.md#grt-divergences).

### git.rs — Git Operations

**Responsibility:** Local git repository operations via git2.

**Public API:**
- `GitRepo::open(path) -> Result<GitRepo>`
- `repo.cherry_pick(commit) -> Result<()>`
- `repo.current_branch() -> Result<String>`
- `repo.is_dirty() -> Result<bool>`
- `repo.resolve_ref(refname) -> Result<Oid>`

**Owns:** `git2::Repository` handle.

**Dependencies:** None (leaf module).

**Design note:** All git2 calls are blocking. Callers must wrap them in `spawn_blocking`. git push to Gerrit's magic refs (`refs/for/branch%topic=...`) may require shelling out to `git` since libgit2's push may not support Gerrit's custom receive-pack options.

### notedb.rs — NoteDb Reader

**Responsibility:** Parse Gerrit metadata stored in git refs (`refs/changes/`, `refs/notes/review`, `refs/meta/config`).

**Public API:**
- `NoteDbReader::new(repo: &GitRepo) -> NoteDbReader`
- `reader.read_change_meta(change_num) -> Result<ChangeMeta>`
- `reader.read_review_notes(commit) -> Result<Vec<ReviewNote>>`

**Owns:** Nothing — borrows `GitRepo`.

**Dependencies:** `git` (borrows `GitRepo`).

### fuzzy.rs — Search

**Responsibility:** Fuzzy matching with nucleo-matcher, query language parsing.

**Public API:**
- `search(candidates: &[Change], query: &str) -> Vec<ScoredChange>`
- `parse_query(input: &str) -> Result<SearchExpr>` — produces an AST
- `compile_query(expr: &SearchExpr) -> Result<SqlFilter>` — AST to SQL

**Owns:** `nucleo_matcher::Matcher` instance.

**Dependencies:** None (leaf module).

**Design note:** Unlike gertty which builds SQLAlchemy expressions directly during parsing, grt separates parsing (producing a `SearchExpr` AST) from query planning (compiling the AST to SQL). This enables validation, optimization, and fuzzy matching integration. See [ref-specs/gertty-search-language.md § grt Divergences](ref-specs/gertty-search-language.md#grt-divergences).

### tui.rs — Terminal UI

**Responsibility:** Event loop, view management, rendering.

**Public API:**
- `tui::run(app: &App) -> Result<()>` — enter TUI mode

**Owns:** Terminal state, view stack, cursor/scroll positions. Borrows `&App` for data access.

**Dependencies:** `app` (borrows `&App`).

**Design note:** Immediate-mode rendering via ratatui (not retained-mode widget tree like gertty/urwid). Application state is the source of truth; views are pure render functions. View stack navigation follows gertty's push/pop model but without widget objects. See [ref-specs/gertty-config-and-ui.md § grt Divergences](ref-specs/gertty-config-and-ui.md#ratatui-vs-urwid-immediate-mode-vs-widget-tree).

## Data Flow

### Inbound (Gerrit → local)

```
Gerrit REST API
     │
     ▼
gerrit.rs ─── HTTP GET /changes/?q=... ───► JSON response
     │
     ▼
Deserialize (serde) into GerritChange structs
     │
     ▼
db.rs ─── upsert_change() ───► SQLite (INSERT ... ON CONFLICT DO UPDATE)
     │
     ▼
Sync event ───► mpsc channel ───► TUI refresh (if running)
```

Sync is driven by a background task (adapted from gertty's sync engine, see [ref-specs/gertty-sync-system.md](ref-specs/gertty-sync-system.md)):
1. A scheduler task maintains a priority queue of sync tasks
2. Worker tasks execute sync operations concurrently (bounded by semaphore)
3. Each sync task fetches data from Gerrit, upserts to SQLite, and notifies the UI
4. Incremental sync uses timestamp watermarks to limit queries to recently-modified changes

### Outbound (local → Gerrit)

```
User action (TUI or CLI)
     │
     ▼
db.rs ─── insert pending_operation row ───► SQLite
     │
     ▼
Upload task (periodic or triggered)
     │
     ▼
db.rs ─── get_pending_operations() ───► read pending ops
     │
     ▼
gerrit.rs ─── POST /changes/{id}/revisions/{rev}/review ───► Gerrit
     │
     ▼
db.rs ─── mark_operation_complete() ───► clear pending flag
     │
     ▼
Enqueue SyncChange to refresh from server
```

Pending operations persist in the database, surviving offline periods and restarts. This is adapted from gertty's `pending_*` flag pattern but consolidated into a single operations table.

### Query (local → display)

```
User input (search query or navigation)
     │
     ▼
fuzzy.rs ─── parse_query() ───► SearchExpr AST
     │
     ├── Field filters ───► compile_query() ───► SQL WHERE clause
     │                                              │
     │                                              ▼
     │                              db.rs ─── query_changes() ───► Vec<Change>
     │
     └── Fuzzy component ───► nucleo_matcher::Matcher::score()
                                              │
                                              ▼
                                    Merge, rank by score, return
     │
     ▼
tui.rs ─── render change list from Vec<ScoredChange>
```

For CLI mode, the final step is formatting and printing instead of rendering.

## Concurrency Model

grt uses a hybrid concurrency model combining tokio-scoped tasks (bounded lifetime, can borrow) with tokio::spawn tasks (static lifetime, independent).

### Channel Topology

```
┌──────────────────────────────────────────────────────────┐
│                   tokio_scoped::scope                      │
│                                                            │
│  ┌─────────────┐  ┌─────────────┐  ┌──────────────────┐ │
│  │ Input Task  │  │ Tick Task   │  │ Sync Event       │ │
│  │ (crossterm  │  │ (interval)  │  │ Receiver         │ │
│  │  EventStream)│ │             │  │ (mpsc::Receiver)  │ │
│  └──────┬──────┘  └──────┬──────┘  └────────┬─────────┘ │
│         │                 │                   │           │
│         └────────┬────────┴───────┬───────────┘           │
│                  ▼                ▼                        │
│         ┌──────────────────────────────┐                  │
│         │  mpsc::channel (TuiEvent)    │                  │
│         └───────────┬──────────────────┘                  │
│                     ▼                                      │
│         ┌──────────────────────┐                          │
│         │  Main Event Loop     │ ◄── borrows &App         │
│         │  tokio::select! {    │                          │
│         │    event = rx.recv() │                          │
│         │  }                   │                          │
│         │  match event {       │                          │
│         │    Input(key) => ... │                          │
│         │    Tick => render()  │                          │
│         │    SyncDone => ...   │                          │
│         │  }                   │                          │
│         └──────────────────────┘                          │
│                                                            │
└──────────────────────────────────────────────────────────┘
                              │
    ┌─────────────────────────┤ (outside scope)
    ▼                         ▼
┌──────────────┐    ┌──────────────────┐
│ Sync Engine  │    │ Periodic Sync    │
│ (JoinSet of  │    │ (tokio::spawn,   │
│  sync tasks) │    │  interval timer) │
│ tokio::spawn │    │                  │
└──────┬───────┘    └────────┬─────────┘
       │                     │
       └──────────┬──────────┘
                  ▼
       ┌────────────────────┐
       │ mpsc::Sender       │
       │ (sync events → TUI)│
       └────────────────────┘
```

### Scoped vs Unscoped Decision Matrix

| Task | Lifetime | Pattern | Rationale |
|------|----------|---------|-----------|
| Keyboard input handler | Scoped | `tokio_scoped` | Borrows `tx`, terminates with TUI |
| Periodic tick generator | Scoped | `tokio_scoped` | Borrows `tx`, terminates with TUI |
| Main TUI event loop | Scoped | `tokio_scoped` | Borrows `&App` and `rx` |
| Background sync engine | Unscoped | `tokio::spawn` | Independent lifetime, owns cloned handles |
| Periodic sync timer | Unscoped | `tokio::spawn` | Continues independently |
| Individual sync tasks | Unscoped | `JoinSet` | Concurrent, bounded by semaphore |
| SQLite operations | Blocking | `spawn_blocking` | rusqlite is synchronous |
| git2 operations | Blocking | `spawn_blocking` | git2 is synchronous |

### Sync Engine Concurrency

The sync engine adapts gertty's single-threaded sync loop into a concurrent architecture:

- A **scheduler task** (`tokio::spawn`) maintains a priority queue (`BinaryHeap`) and a deduplication set (`HashSet`)
- Sync task submissions arrive via `mpsc::Sender<SyncTask>`
- The scheduler dispatches tasks to a **worker pool** bounded by `tokio::sync::Semaphore` (e.g., 4 concurrent HTTP requests)
- Each worker executes one sync task: HTTP fetch → deserialize → `spawn_blocking` for DB upsert → notify UI
- Completion notifications flow back via `mpsc::Sender<SyncEvent>` to the TUI event loop

## Startup and Shutdown

### Initialization Order

```
1. Parse CLI args (clap)
2. Initialize tracing (log file + stderr)
3. Load config from TOML file
     ├── Validate config
     ├── Resolve paths (~/ expansion, XDG directories)
     └── Prompt for password if not in config and not in keyring
4. Open SQLite database
     ├── Create if not exists
     ├── Run migrations (check user_version pragma)
     └── Enable foreign keys (PRAGMA foreign_keys = ON)
5. Construct App { config, database, gerrit_client, git_repo }
6. Route to mode:
     ├── CLI: execute command, print result, exit
     └── TUI:
          ├── Initialize terminal (ratatui::init)
          ├── Spawn background sync engine (tokio::spawn)
          ├── Enter tokio_scoped scope
          │    ├── Spawn input handler
          │    ├── Spawn tick generator
          │    └── Run main event loop
          └── Scope exits → all scoped tasks complete
```

### Graceful Shutdown

**TUI mode:**
1. User presses quit key (or `Ctrl-C` triggers `tokio::signal::ctrl_c()`)
2. Main event loop breaks, scope begins unwinding
3. All scoped tasks (input, tick) complete because the scope awaits them
4. Background sync engine receives shutdown signal via `CancellationToken`
5. Sync engine cancels in-flight sync tasks (`JoinSet::abort_all()`)
6. Sync engine drains — any pending outbound operations remain in the database for next run
7. Terminal state restored (`ratatui::restore()`)
8. Database connection closed (via `Drop`)

**CLI mode:**
1. Command completes (success or error)
2. `App` dropped, database connection closed
3. Process exits with code 0 (success) or 1 (error)

### Fatal vs Recoverable Startup Errors

| Error | Severity | Behavior |
|-------|----------|----------|
| Config file not found | Fatal | Print path and exit |
| Config file invalid TOML | Fatal | Print parse error with line/column and exit |
| Database open failure | Fatal | Print path and OS error and exit |
| Migration failure | Fatal | Print migration version and error and exit |
| Gerrit server unreachable | Recoverable (TUI) | Start in offline mode, retry in background |
| Gerrit server unreachable | Recoverable (CLI sync) | Print error and exit with code 1 |
| Git repo not found | Recoverable | Operations requiring git will fail individually |
| Terminal initialization failure | Fatal | Print error to stderr and exit |

## Error Propagation

### Cross-Module Error Flow

```
Leaf modules (db, gerrit, git, notedb, fuzzy)
     │
     │  Return Result<T, specific_error> or Result<T, anyhow::Error>
     │  Attach context: .context("loading change 12345 from database")
     ▼
app.rs (orchestrator)
     │
     │  Catches errors, decides: retry? degrade? propagate?
     │  Attaches higher-level context: .context("syncing project openstack/nova")
     ▼
Interface layer (tui.rs or main.rs CLI)
     │
     ├── TUI: Display error in status bar, do not crash
     │         Catch in event handler, set app.status_message
     │         User sees: "Error: syncing project openstack/nova: HTTP 401 Unauthorized"
     │
     └── CLI: Print to stderr, exit with code 1
              eprintln!("Error: {:#}", err)  // anyhow's alternate display shows chain
              std::process::exit(1)
```

### anyhow vs Typed Errors

grt uses a two-tier error strategy:

- **`anyhow::Result<T>`** for most functions — ergonomic, chainable context, automatic backtrace. Used everywhere except where callers need to match on specific errors.
- **`thiserror` enums** at module boundaries where callers need to distinguish error types. Candidates:
  - `GerritError { AuthFailed, NotFound, ServerError(u16), NetworkError, ... }` — so the sync engine can distinguish retryable errors from permanent failures
  - `DbError { MigrationFailed, CorruptSchema, ... }` — so startup can report specific database issues
  - `SearchError { SyntaxError { span, message }, ... }` — so the UI can show diagnostic output

### TUI Error Handling Pattern

In TUI mode, errors must never crash the application. The pattern:

```
// In the event loop:
match app.sync_change(id).await {
    Ok(change) => self.display_change(change),
    Err(e) => self.status_message = format!("{:#}", e),
}
```

The status bar shows the most recent error. Errors from background sync tasks flow through the `mpsc::Sender<SyncEvent>` channel as `SyncEvent::Error(String)` events.

### CLI Error Handling Pattern

In CLI mode, errors propagate via `?` to `main()`, which uses anyhow's display:

```rust
fn main() -> anyhow::Result<()> {
    // ... setup ...
    app.run_command(args)?;
    Ok(())
}
```

The `{:#}` display format produces a chain like: `Error: syncing project: HTTP request failed: connection refused`.

For structured exit codes in scripting contexts, specific commands can catch typed errors and map them:

```rust
match app.sync().await {
    Ok(report) => { print_report(report); Ok(()) }
    Err(e) if e.downcast_ref::<GerritError>().map_or(false, |e| e.is_auth()) => {
        eprintln!("Authentication failed. Check credentials.");
        std::process::exit(2)
    }
    Err(e) => Err(e),
}
```
