# Rust Conventions for grt

Single source of truth for coding standards. Derived from [tech-stack.md](tech-stack.md) selections.

## Error Handling

- Use `anyhow::Result<T>` for all fallible functions
- Attach context with `.context("what was being attempted")` — describe the operation, not the error
- In public API boundaries, consider `thiserror` enums for errors callers need to match on
- In TUI mode: catch errors in event handlers, display in status bar, never crash the UI
- In CLI mode: print to stderr, exit with non-zero code

```rust
use anyhow::{Context, Result};

fn load_change(id: &str) -> Result<Change> {
    db.get_change(id)
        .await
        .context("loading change from database")?
}
```

## Async Patterns

- All I/O is async via Tokio
- Use `tokio-scoped` for tasks that borrow from parent (UI-coordinated tasks)
- Use `tokio::spawn` only for `'static` long-lived background tasks
- Use `tokio::select!` for event multiplexing in the main loop
- Use `mpsc` channels for communication between tasks

```rust
// Scoped: borrows tx, guaranteed to complete
tokio_scoped::scope(|scope| {
    scope.spawn(async { tx.send(event).await });
});

// Unscoped: owns data, independent lifetime
let tx = tx.clone();
tokio::spawn(async move { tx.send(event).await });
```

## Module Layout

- One module per file, matching the structure in `ai/context.md`
- `pub` items at top of file, private helpers below
- `#[cfg(test)] mod tests` at bottom of each module
- Integration tests in `tests/` directory

## Naming

- Types: `PascalCase` — `Change`, `GerritClient`, `NoteDbReader`
- Functions/methods: `snake_case` — `fetch_changes`, `upsert_change`
- Constants: `SCREAMING_SNAKE_CASE` — `MAX_RESULTS`, `DEFAULT_TIMEOUT`
- Modules: `snake_case` — `gerrit`, `notedb`, `fuzzy`

## Structs and Data

- Derive `Debug` on all types
- Derive `Clone` only when needed
- Use `serde::{Serialize, Deserialize}` for types crossing I/O boundaries
- Prefer owned `String` over `&str` in long-lived structs
- Use newtype wrappers for IDs: `struct ChangeId(String)`

## Database (sqlx)

- Use compile-time checked queries with `sqlx::query!` / `sqlx::query_as!`
- All queries are parameterized (never string-interpolate SQL)
- Use `INSERT ... ON CONFLICT DO UPDATE` for upserts
- Wrap multi-step operations in transactions

## Git Operations (gix)

- Always handle gix errors — never unwrap git operations
- Close repository handles when done (they hold file locks)
- Validate ref names before use
- Wrap all gix calls in `tokio::task::spawn_blocking` — gix's API is blocking

## Logging (tracing)

- Use `#[instrument]` on public functions for automatic span creation
- Use structured fields: `info!(change_id = %id, "synced change")`
- Log levels: ERROR (broken), WARN (degraded), INFO (milestones), DEBUG (flow), TRACE (data)
- Never log secrets or credentials

## Testing

- Unit tests: `#[cfg(test)]` in each module
- Integration tests: `tests/` directory
- Use `tempfile` for filesystem isolation
- Use `mockito` for HTTP mocks
- Use `serial_test` for tests sharing SQLite
- Prefer `assert_eq!` with descriptive messages over bare `assert!`

## Formatting and Linting

- `cargo fmt` — default rustfmt settings
- `cargo clippy` — treat warnings as errors in CI
- No `#[allow(unused)]` without a comment explaining why
