# Tech Stack Design Document: grt

**Version:** 0.2  
**Date:** February 2026  
**Status:** Adopted  
**Supersedes:** v0.1

## Design Principles

1. **Async-first.** All I/O is non-blocking at the call site. The main event loop never waits on a thread.
2. **Local-first.** The SQLite database is the single source of truth for the UI. The user never waits on a network round-trip to view data.
3. **Pure-Rust preferred.** C dependencies are avoided where a mature pure-Rust alternative exists. This minimises libc coupling and simplifies cross-compilation. Where a C dependency is unavoidable, it must be clearly justified.
4. **No vendoring.** All dependencies are sourced from crates.io as declared in `Cargo.toml`. No vendored copies, no patched forks, no `[patch]` overrides.
5. **Dual interface.** The same core logic serves both TUI (interactive) and CLI (scriptable) modes. No business logic lives in either interface layer.
6. **Observable.** Structured logging and span-propagating traces are first-class, not an afterthought.

---

## Language and Runtime

### Rust (Edition 2021)

**Role:** Primary programming language.

Pure-Rust is the default for all dependency selection. Memory safety without a garbage collector, expressive async/await, zero-cost abstractions, and excellent cross-compilation make it the right choice for a local-first, async-first CLI/TUI tool.

### Tokio (v1.x)

**Role:** Async runtime and task scheduler.

The de facto standard for async Rust. Provides the multi-threaded work-stealing scheduler, channels, timers, sync primitives, and `spawn_blocking` for offloading blocking operations. Key features used:

- `tokio::spawn` — long-lived background tasks with `'static` lifetime
- `tokio::task::spawn_blocking` — wrapping unavoidably blocking operations (git)
- `tokio::select!` — event multiplexing in the main TUI loop
- `tokio::time::interval` — periodic tick generation for TUI and background sync
- `mpsc` / `oneshot` channels — cross-task communication

### tokio-scoped (v0.2)

**Role:** Structured concurrency for TUI-lifetime tasks.

Enables borrowing in spawned tasks (no `'static` requirement) with a guarantee that all tasks complete before the scope exits. Used for input handlers, tick generators, and the main event loop — tasks that should terminate when the TUI session ends.

**When to use:** Tasks that borrow app state, tasks that must terminate with the TUI.  
**When not to use:** Long-running background tasks that outlive the TUI session.

---

## User Interface Layer

### ratatui (v0.28)

**Role:** Terminal User Interface framework.

Pure-Rust, immediate-mode rendering (React-like). No ncurses dependency. Composable widget system, flexible layout constraints, event-driven re-renders.

**Key integration points:**
- `ratatui::init()` / `restore()` — terminal setup and cleanup with panic hooks
- Draw functions take an immutable reference to application state
- Re-renders are event-driven, not continuous-polling

### crossterm (v0.28)

**Role:** Cross-platform terminal manipulation.

Pure Rust. Works on Linux, macOS, and Windows. Provides raw mode, alternate screen, cursor control, and an async `EventStream`. Integrated with `tokio::select!` for keyboard/mouse event multiplexing.

### clap (v4.x)

**Role:** Command-line argument parsing.

Derive-based declarative API with automatic help generation, shell completion support, subcommand handling, and type-safe argument parsing. No runtime reflection.

**Command structure:**
```
grt [OPTIONS] [COMMAND]
  --config <PATH>
  --repo <PATH>
  --gerrit-server <URL>
  --log-level <LEVEL>
  --log-file <PATH>

Commands:
  tui           # Launch TUI (default when no command given)
  list          # List changes
  sync          # Sync from Gerrit
  search        # Fuzzy search
  show          # Show change details
  cherry-pick   # Cherry-pick a change
  init          # Initialise local database
```

---

## Data Layer

### sqlx (v0.8) with SQLite

**Role:** Async local database — the single source of truth for the UI.

sqlx is selected over rusqlite because it provides a native async SQLite driver compatible with Tokio, eliminating the need for `spawn_blocking` on database calls. This directly serves the async-first principle. Compile-time SQL query verification catches schema/query mismatches before they reach runtime.

**Why not rusqlite:** rusqlite is synchronous. Every call would require `spawn_blocking`, introducing thread-pool overhead on every query, adding spans that are invisible by default, and coupling the data layer tightly to the blocking thread-pool capacity. sqlx avoids all of this.

**Feature flags:**
```toml
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "migrate"] }
```

**Schema design:**
```sql
CREATE TABLE changes (
    id          TEXT PRIMARY KEY,
    subject     TEXT NOT NULL,
    status      TEXT NOT NULL CHECK(status IN ('NEW', 'MERGED', 'ABANDONED')),
    owner       TEXT NOT NULL,
    branch      TEXT NOT NULL,
    commit_sha  TEXT NOT NULL,
    project     TEXT NOT NULL,
    created_at  INTEGER NOT NULL,  -- Unix timestamp
    updated_at  INTEGER NOT NULL
);
CREATE INDEX idx_changes_status   ON changes(status);
CREATE INDEX idx_changes_updated  ON changes(updated_at);
CREATE INDEX idx_changes_owner    ON changes(owner);
CREATE INDEX idx_changes_branch   ON changes(branch);

CREATE TABLE patchsets (
    id          TEXT PRIMARY KEY,
    change_id   TEXT NOT NULL REFERENCES changes(id) ON DELETE CASCADE,
    number      INTEGER NOT NULL,
    commit_sha  TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE pending_operations (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    op_type     TEXT NOT NULL,     -- 'review', 'abandon', 'cherry_pick'
    change_id   TEXT NOT NULL,
    payload     TEXT NOT NULL,     -- JSON-encoded op parameters
    created_at  INTEGER NOT NULL,
    attempts    INTEGER NOT NULL DEFAULT 0
);
```

Schema version is tracked via SQLite's `user_version` pragma. Migrations are embedded in the binary via sqlx's `migrate!` macro and run at startup.

**Data flow (inbound):**
1. Fetch from Gerrit REST API → deserialise JSON
2. Transform to internal `Change` model
3. Upsert to SQLite (`INSERT ... ON CONFLICT DO UPDATE`)
4. Emit sync event to TUI channel

**Query discipline:** SQL filters execute first using indexed columns (`status`, `owner`, `branch`, `updated_at`), dramatically reducing the candidate set before fuzzy matching runs. The 1000-row hardcoded limit used in v0.1 is removed in favour of this SQL-first approach.

---

## Git Integration

### gix (gitoxide, v0.x)

**Role:** Git repository operations — ref reading, object access, NoteDb traversal, cherry-pick.

`gix` is selected over `git2` (libgit2 bindings) because it is pure Rust with no C dependency, eliminating the only significant libc coupling in the stack. It is the same library now used by `cargo` itself for git operations.

**Important constraint:** gix's library API is blocking. Despite async transport support existing as a feature-flag demonstration, the core object-access and repository-manipulation APIs are synchronous. All `gix` calls in grt are therefore wrapped in `tokio::task::spawn_blocking`. This is an honest tradeoff: we remove the C dependency entirely but retain the `spawn_blocking` pattern. Spans must be explicitly propagated into `spawn_blocking` closures — see the Observability section.

**Primary use cases:**
- Read repository status (current branch, dirty state)
- Resolve and read `refs/changes/XX/NNNN/meta` — NoteDb change metadata
- Read `refs/notes/review` — review comments stored as Git notes
- Apply cherry-picks by composing diff and apply operations

**Cherry-pick note:** gix does not yet expose a single high-level `cherry_pick()` call equivalent to `git cherry-pick`. The operation must be composed from lower-level gix APIs: resolve the commit, compute the diff against its parent, apply the patch to the working tree, create a new commit. If this proves unworkable during implementation, falling back to shelling out to the system `git` binary for cherry-pick specifically is acceptable as a pragmatic escape hatch.

**No vendoring:** gix is taken from crates.io. Feature flags should be carefully scoped to only the capabilities needed:
```toml
gix = { version = "0.x", default-features = false, features = [
    "revision",   # commit graph traversal
    "blob-diff",  # diff computation for cherry-pick
    "index",      # index read/write
] }
```

---

## External Integration

### reqwest (v0.12)

**Role:** Async HTTP client for the Gerrit REST API.

**Feature flags — critical for pure-Rust and static linking:**
```toml
reqwest = { version = "0.12", default-features = false, features = [
    "rustls-tls",  # pure-Rust TLS — no OpenSSL, no libssl dependency
    "json",        # serde_json integration
    "gzip",        # response decompression
] }
```

`openssl` and `native-tls` features must remain disabled. Using `rustls-tls` means TLS is implemented entirely in Rust (via the `rustls` and `ring`/`aws-lc-rs` crates), with no dependency on the system OpenSSL.

**Gerrit-specific behaviour:**
- Strip the Gerrit XSSI prefix (`)]}'\n`) from all responses before JSON deserialisation
- Implement retry with exponential backoff for transient errors (5xx, connection errors). Retry logic depends on `GerritError::is_retryable()` — see Error Handling
- `ETag` / `If-None-Match` conditional requests for polling efficiency (future)

**Authentication:** To be designed. Candidates: HTTP Digest, HTTP Basic over HTTPS, cookie-based. The `GerritClient` public API must accommodate auth credentials from construction; the exact mechanism will be determined during NoteDb validation work.

---

## Search and Filtering

### nucleo-matcher (v0.3)

**Role:** Fuzzy scoring algorithm.

Same algorithm as fzf. Pure Rust. Provides scoring and case-insensitive matching. Used only on the post-SQL filtered candidate set — not on the full change corpus.

**Search pipeline:**
1. Parse query with `fuzzy::parse_query()` → `SearchExpr` AST
2. Compile field filters to SQL: `fuzzy::compile_query()` → `SqlFilter`
3. Execute SQL against SQLite, applying indexed filters (status, owner, branch, etc.)
4. Score remaining candidates with nucleo-matcher against the free-text component
5. Sort by score, return `Vec<ScoredChange>`

The AST/SQL separation enables validation at parse time, optimisation, and clean integration of the fuzzy component. Advanced filter syntax (`status:NEW`, `owner:alice`) maps directly to indexed SQL columns.

---

## Error Handling

### thiserror (v1.x)

**Role:** Typed error enums at module boundaries.

Used in all library modules (`db`, `gerrit`, `git`, `notedb`, `fuzzy`) where callers need to distinguish error variants. `thiserror` provides derive-based `std::error::Error` implementations with zero overhead.

Typed error enums must be defined before implementation begins — particularly `GerritError`, which is required by the retry logic:

```rust
#[derive(Debug, thiserror::Error)]
pub enum GerritError {
    #[error("authentication failed: {0}")]
    AuthFailed(String),
    #[error("change not found: {0}")]
    NotFound(String),
    #[error("server error {status}: {body}")]
    ServerError { status: u16, body: String },
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
}

impl GerritError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::ServerError { status, .. } if *status >= 500)
            || matches!(self, Self::Network(_))
    }
}
```

### anyhow (v1.x)

**Role:** Ergonomic error propagation in orchestration code.

Used in `app.rs` (the orchestrator) and `main.rs` (the CLI entry point) for propagating and contextualising errors from leaf modules. `.context()` / `.with_context()` attach human-readable context at each layer. The `{:#}` alternate display format renders the full error chain.

```rust
// In app.rs — attach context at the orchestration layer
client.get_change(id).await
    .context("fetching change from Gerrit")?;

// CLI error display in main.rs
eprintln!("Error: {:#}", err);
```

### miette (v7.x) with `fancy` feature

**Role:** Diagnostic error reporting for structured logging output.

**Scope:** miette is used for errors reported via the tracing/logging system — errors that include source location, span metadata, or actionable help text intended for developers and operators reading logs. It is not used for user-facing CLI/TUI output.

Specifically:
- **`SearchError { SyntaxError { span, message } }`** — query parse failures that should appear in diagnostic logs with the span of the invalid input highlighted
- **`ConfigError`** — TOML config file parse failures with file/line/column context
- **`MigrationError`** — schema migration failures with migration version context

miette's `Diagnostic` trait is derived alongside `thiserror::Error`. The `fancy` feature flag is enabled only in the binary crate (`main.rs`), not in any library crate, to avoid pulling display dependencies into libraries:

```toml
# In the binary crate only:
miette = { version = "7", features = ["fancy"] }
# In library crates:
miette = { version = "7" }  # no fancy
```

```rust
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("search query syntax error")]
#[diagnostic(code(grt::search::syntax), help("check query syntax: status:NEW owner:alice"))]
pub struct SearchSyntaxError {
    #[source_code]
    pub src: String,
    #[label("unexpected token")]
    pub span: miette::SourceSpan,
}
```

---

## Observability

### tracing (v0.1) + tracing-subscriber (v0.3)

**Role:** Structured event logging and span-based execution tracing.

All significant operations are instrumented with `#[instrument]` or manual `span!` calls. Structured fields (key-value pairs) accompany all log events.

**Span propagation convention — mandatory:**
Tokio does not automatically propagate tracing spans into spawned tasks. Every `tokio::spawn` call and every `spawn_blocking` call must explicitly carry the parent span:

```rust
// tokio::spawn — propagate via Instrumented future
let span = info_span!("sync_change", change_id = %id, project = %project);
tokio::spawn(
    async move { /* ... sync work ... */ }.instrument(span)
);

// spawn_blocking — propagate via explicit entry
let span = info_span!("git_cherry_pick", commit = %sha);
tokio::task::spawn_blocking(move || {
    let _guard = span.enter();
    // ... git2/gix work ...
})
```

This is enforced in code review. Without it, background task failures produce disconnected spans that are very difficult to correlate with the triggering user action.

**TUI mode logging:** When TUI mode is active, log output to the terminal is destructive (overwrites the TUI). The startup sequence detects TUI mode and requires a log file to be configured. If no `--log-file` is provided in TUI mode, grt warns before entering the alternate screen and disables console output entirely, rather than silently swallowing log output.

**Log levels:**
- `ERROR` — unrecoverable failures requiring operator attention
- `WARN` — retried operations, degraded-mode decisions, unexpected but handled conditions
- `INFO` — sync completion with counts, startup/shutdown lifecycle events
- `DEBUG` — per-request HTTP events, per-query SQL, individual task dispatch
- `TRACE` — nucleo-matcher scoring, raw response bodies, internal state transitions

**Configured via:** `RUST_LOG` environment variable and `--log-level` CLI flag.

### tracing-subscriber (v0.3)

**Role:** Log output configuration.

Two output formats:
- **Human-readable** (development default): `EnvFilter` + `fmt` layer, colourised, with timestamp and level
- **JSON** (production/CI): `fmt::json()` layer for structured log aggregation (ELK, Loki, etc.)

Selected via `--log-json` flag or `GRT_LOG_JSON=1` environment variable.

### tracing-appender (v0.2)

**Role:** Non-blocking log file output with daily rotation.

Writes to a dedicated background thread. Prevents log I/O from blocking the async event loop. Integrates with `tracing-subscriber` as a layer alongside the console layer (when not in TUI mode).

### tracing-error (v0.2)

**Role:** `SpanTrace` capture — attaches the active span stack to errors.

When an error originates in a background sync task and is sent via `mpsc` channel to the TUI for display, `SpanTrace` preserves the tracing context at the error site. Without this, the error message in the TUI status bar or log file has no context about which change, project, or HTTP request produced it.

```rust
use tracing_error::SpanTrace;

#[derive(Debug, thiserror::Error)]
#[error("sync failed for change {change_id}: {source}")]
pub struct SyncError {
    pub change_id: String,
    #[source]
    pub source: GerritError,
    pub span_trace: SpanTrace,  // captured at error site
}
```

Integrates naturally with `color-eyre` for developer-facing error display.

### color-eyre (v0.6)

**Role:** Enhanced error display for developer-facing CLI output.

`color-eyre` replaces `anyhow`'s plain-text chain display in CLI mode with colourised, multi-section output that includes:
- The `SpanTrace` from `tracing-error` — where in the async/span hierarchy the error occurred
- A `BackTrace` if `RUST_BACKTRACE=1` is set
- Colourised error chain with each cause clearly separated

`color-eyre` is configured in `main()` via `color_eyre::install()`. It wraps `anyhow`'s `Report` type and is compatible with the `?` propagation pattern already in use. Library modules continue to use `thiserror` and `anyhow` — `color-eyre` is a display concern in the binary entry point only.

```rust
// In main.rs
fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    // ...
}
```

---

## Configuration

### toml (v0.8) + serde (v1.x)

**Role:** Config file parsing.

A strongly-typed `Config` struct is deserialised from a TOML file at startup. `serde`'s derive macros handle the mapping. Errors from TOML parsing are wrapped in `ConfigError` with miette span information pointing to the problematic location in the config file.

**Config file location:** XDG base directory convention. `$XDG_CONFIG_HOME/grt/config.toml` on Linux, `~/.config/grt/config.toml` as the default.

### dirs (v5.x)

**Role:** Platform-appropriate path resolution.

Pure Rust. Resolves XDG config home, data home, and cache home. Used for default config file path, default database path, and default log file path.

---

## Progress Reporting (CLI mode)

### indicatif (v0.17)

**Role:** Progress bars and spinners for long-running CLI operations.

Long-running CLI commands (`sync`, `init`) must not produce silent output. `indicatif` provides spinner/progress-bar widgets that render to stderr without interfering with stdout output. Integrates with `tracing` via `indicatif-log-bridge` so that log messages interleave cleanly with progress output.

---

## Testing Infrastructure

### Core test crates

**tokio-test (v0.4)** — testing async code, mock time for interval/timeout testing.

**tempfile (v3)** — temporary directories and files for database isolation. Automatic cleanup on drop.

**mockito (v1.5)** — HTTP mock server for testing `gerrit.rs` without a live Gerrit instance. Each test gets its own server instance on a random port.

**serial_test (v3)** — serialise tests that cannot safely run concurrently. Used sparingly — prefer test isolation via `tempfile` over serialisation.

**proptest (v1)** — property-based testing with generated inputs. Primary use case: `fuzzy.rs` search algorithm must never panic on arbitrary Unicode input.

**test-case (v3)** — parameterised test cases to reduce test boilerplate.

**criterion (v0.5)** — micro-benchmarking for performance-sensitive paths. Benchmarks the SQLite query path (with realistic dataset sizes) and the nucleo-matcher scoring path. These are required before the project is considered feature-complete, because the "< 100ms render time" success criterion has no other mechanism to detect regressions.

### Test organisation

```
tests/
├── common/
│   └── mod.rs              # TestContext, fixtures, shared setup
├── db_tests.rs             # Database schema, queries, migrations
├── fuzzy_tests.rs          # Search algorithm, query parser
├── gerrit_tests.rs         # Gerrit client against mockito server
├── git_tests.rs            # gix operations against temp repositories
├── app_integration_tests.rs # End-to-end workflows through App
└── property_tests.rs       # proptest generative tests
benches/
├── db_query.rs             # criterion benchmarks for SQLite queries
└── fuzzy_search.rs         # criterion benchmarks for nucleo-matcher
```

**Test categories:**
- **Unit tests** — single function/module, in `src/` files under `#[cfg(test)]`
- **Integration tests** — component interaction, in `tests/`
- **Property tests** — generative edge-case testing, in `tests/property_tests.rs`
- **Mock tests** — Gerrit HTTP integration with mockito, in `tests/gerrit_tests.rs`
- **Benchmarks** — performance regression detection, in `benches/`

---

## Module Structure

```
src/
├── main.rs      CLI entry point, logging setup, color-eyre install
├── app.rs       App struct, business logic orchestration
├── db.rs        SQLite schema, queries, migrations (sqlx)
├── gerrit.rs    Gerrit REST client, auth, retry (reqwest + rustls)
├── git.rs       gix operations, cherry-pick, ref resolution
├── notedb.rs    NoteDb ref parsing (gix)
├── fuzzy.rs     Search query parser, SQL compiler, nucleo-matcher scoring
├── config.rs    Config struct, TOML loading, ConfigError (miette)
└── tui.rs       ratatui event loop, views, rendering
```

### Module dependency rules

- Leaf modules (`db`, `gerrit`, `git`, `notedb`, `fuzzy`, `config`) have no dependencies on each other
- `notedb` borrows from `git` (shares the `gix::Repository` handle)
- `app` owns all leaf modules and mediates all interaction between them
- `tui` borrows `&App` (via `tokio-scoped`) for all data access
- `main` depends on `app`, `tui`, and all CLI-reachable modules

---

## Crate Summary

| Crate | Version | Role | Pure Rust | C dependency |
|-------|---------|------|-----------|--------------|
| tokio | 1.x | Async runtime | ✓ | — |
| tokio-scoped | 0.2 | Structured concurrency | ✓ | — |
| ratatui | 0.28 | TUI framework | ✓ | — |
| crossterm | 0.28 | Terminal control | ✓ | — |
| clap | 4.x | CLI argument parsing | ✓ | — |
| sqlx | 0.8 | Async SQLite (no spawn_blocking) | ✓ | — |
| gix | 0.x | Git operations (no libgit2) | ✓ | — |
| reqwest | 0.12 | HTTP client (rustls-tls only) | ✓* | — |
| rustls | 0.x | TLS implementation | ✓ | — |
| nucleo-matcher | 0.3 | Fuzzy search algorithm | ✓ | — |
| thiserror | 1.x | Typed error enums | ✓ | — |
| anyhow | 1.x | Ergonomic error propagation | ✓ | — |
| miette | 7.x | Diagnostic error reporting (logging) | ✓ | — |
| tracing | 0.1 | Structured logging and spans | ✓ | — |
| tracing-subscriber | 0.3 | Log output configuration | ✓ | — |
| tracing-appender | 0.2 | Non-blocking log file rotation | ✓ | — |
| tracing-error | 0.2 | SpanTrace capture | ✓ | — |
| color-eyre | 0.6 | Enhanced CLI error display | ✓ | — |
| serde | 1.x | Serialisation framework | ✓ | — |
| serde_json | 1.x | JSON (Gerrit API) | ✓ | — |
| toml | 0.8 | Config file parsing | ✓ | — |
| dirs | 5.x | XDG path resolution | ✓ | — |
| indicatif | 0.17 | CLI progress bars | ✓ | — |
| tokio-test | 0.4 | Async test utilities | ✓ | — |
| tempfile | 3.x | Test isolation | ✓ | — |
| mockito | 1.5 | HTTP mock server | ✓ | — |
| serial_test | 3.x | Test serialisation | ✓ | — |
| proptest | 1.x | Property-based testing | ✓ | — |
| test-case | 3.x | Parameterised tests | ✓ | — |
| criterion | 0.5 | Micro-benchmarking | ✓ | — |

*reqwest uses `ring` (C-adjacent assembly) or `aws-lc-rs` for cryptographic primitives when the `rustls-tls` feature is active. This is the standard pure-Rust TLS path and does not introduce a libc dependency in the same sense as OpenSSL.

### Intentionally absent crates

**rusqlite** — replaced by sqlx. rusqlite is synchronous, requiring `spawn_blocking` on every database call. sqlx's async SQLite driver is the correct choice for an async-first codebase.

**git2** (libgit2 bindings) — replaced by gix. git2 requires libgit2 (C), which is the primary libc coupling in the original stack. gix is pure Rust and is now used by cargo itself.

**openssl / native-tls** — excluded. reqwest is configured with `rustls-tls` and `default-features = false` to prevent accidental OpenSSL reintroduction. `openssl-sys` should be added to the `bans.deny` list in `deny.toml`.

---

## Concurrency Model

### Scoped vs unscoped tasks

| Task | Lifetime | Pattern | Rationale |
|------|----------|---------|-----------|
| Keyboard input handler | TUI session | `tokio_scoped` | Borrows `tx`, must terminate with TUI |
| Periodic tick generator | TUI session | `tokio_scoped` | Borrows `tx`, must terminate with TUI |
| Main TUI event loop | TUI session | `tokio_scoped` | Borrows `&App` |
| Background sync engine | `'static` | `tokio::spawn` | Independent lifetime, owns cloned handles |
| Periodic sync timer | `'static` | `tokio::spawn` | Independent of TUI |
| Individual sync workers | `'static` | `JoinSet` | Concurrent, bounded by `Semaphore` |
| git operations (gix) | bounded | `spawn_blocking` | gix API is blocking; tokio offloads to thread pool |

**Span propagation is mandatory for all `tokio::spawn` and `spawn_blocking` calls.** See the Observability section for the required pattern.

### spawn_blocking usage

gix operations are the only remaining `spawn_blocking` site. sqlx eliminates `spawn_blocking` for all database calls. The async-first principle is met: the event loop never blocks, and the only thread-pool overhead is for git operations, which are inherently I/O and CPU intensive enough to justify it.

---

## Error Strategy at a Glance

```
Leaf modules (db, gerrit, git, notedb, fuzzy, config)
    │
    │  thiserror enums — typed, matchable
    │  miette::Diagnostic on ConfigError, SearchSyntaxError — for diagnostic logging
    │  tracing-error SpanTrace — captured at error site
    ▼
app.rs — orchestrator
    │
    │  anyhow::Result<T> — context attachment (.context()), ergonomic propagation
    ▼
Interface layer
    │
    ├── TUI: display in status bar, do not crash
    │         SyncEvent::Error(String) via mpsc channel
    │         NarratableReportHandler or plain .to_string() for single-line status
    │
    └── CLI: color-eyre Report — colourised chain + SpanTrace + backtrace
              color_eyre::Result<()> in main()
```

---

## Known Constraints and Open Questions

### gix cherry-pick maturity
gix does not yet expose a high-level `cherry_pick()` API. The operation must be composed from diff and apply primitives. If this is unworkable, shelling out to the system `git` binary for cherry-pick only is the pragmatic fallback. This should be validated early — before the rest of `git.rs` is built around the gix API.

### Authentication
Not yet designed. The choice of auth mechanism (HTTP Digest, HTTP Basic, cookie-based) affects the `GerritClient` constructor API. This must be resolved before any code that calls `GerritClient::new()` is written, to avoid a later breaking refactor.

### keyring for credential storage
The `keyring` crate (OS keychain integration) has a dependency on `libdbus-sys` (D-Bus) on Linux. This introduces a C dependency and a system service dependency that conflicts with the pure-Rust goal. Options: (a) accept dynamic linking for the `keyring` crate specifically, (b) implement a fallback file-based credential store encrypted with a user passphrase, or (c) require credentials to be supplied via environment variable or config file. This decision is deferred until the authentication mechanism is chosen.

### NoteDb format
Gerrit's NoteDb format (refs/changes/, refs/notes/review, refs/meta/config) is not formally documented. The format must be validated against a real Gerrit instance before the data model is finalised. This work should happen during the first implementation sprint, not after.

### tokio-scoped maintenance
`tokio-scoped` is a small crate with limited recent activity. An alternative achieving similar structured lifetime semantics using only tokio primitives is `tokio::task::JoinSet` combined with `CancellationToken` from `tokio-util`. This trades the ergonomics of borrowing in task bodies for freedom from the scoped crate. Evaluate during early implementation; switch if `tokio-scoped` presents compatibility issues with future tokio releases.
