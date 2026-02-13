# grt Error Handling

**Related docs:** `architecture.md`, `../adopted/tech-stack.md`
**Source code:** All modules, particularly `crates/grt/src/gerrit.rs` and `crates/grt/src/main.rs`
**Status:** Draft

## Overview

grt uses a two-tier error strategy:

1. **`thiserror` enums** at the Gerrit client boundary (`GerritError`) for typed error classification, retryability checks, and exit code mapping.
2. **`anyhow`** throughout the rest of the application for ergonomic error propagation with context chains.

## Typed Errors: GerritError

The `GerritError` enum in `gerrit.rs` classifies Gerrit REST API failures:

```rust
#[derive(Debug, thiserror::Error)]
pub enum GerritError {
    #[error("authentication failed (HTTP {status})")]
    AuthFailed { status: u16 },

    #[error("not found (HTTP 404)")]
    NotFound,

    #[error("server error (HTTP {status}): {body}")]
    ServerError { status: u16, body: String },

    #[error("network error: {0}")]
    Network(String),
}
```

### Retryability Classification

Each error variant has a retryability classification used by the retry logic:

```rust
impl GerritError {
    pub fn is_retryable(&self) -> bool {
        match self {
            GerritError::ServerError { status, .. } => *status >= 500,
            GerritError::Network(_) => true,
            _ => false,
        }
    }
}
```

- **Retryable:** 5xx server errors, network failures (connection reset, timeout, DNS)
- **Not retryable:** 401/403 auth failures, 404 not found, 4xx client errors

### Error Flow

The internal `get_once()` method returns `Result<String, GerritError>` (typed). The public `get()` method wraps this with retry logic and converts to `anyhow::Result<String>` via `.context()`, preserving the `GerritError` in the error chain for downstream `downcast_ref`.

## Exit Code Mapping

The `main()` function maps errors to exit codes via `exit_code_for_error()`:

```rust
fn exit_code_for_error(err: &anyhow::Error) -> i32 {
    if let Some(gerrit_err) = err.downcast_ref::<GerritError>() {
        return match gerrit_err {
            GerritError::AuthFailed { .. } => 1,
            GerritError::NotFound => 1,
            GerritError::ServerError { .. } => 1,
            GerritError::Network(_) => 40,
        };
    }
    let msg = format!("{err:#}");
    if msg.contains("git config") || msg.contains("no Gerrit host configured") { return 128; }
    if msg.contains("argument") || msg.contains("CHANGE,PS") || msg.contains("malformed") { return 3; }
    if msg.contains("hook") { return 2; }
    1
}
```

| Code | Meaning | Source |
|------|---------|-------|
| 0 | Success | Normal exit |
| 1 | Generic error | Default for unclassified errors, auth failures, server errors |
| 2 | Hook-related error | Hook installation failures |
| 3 | Malformed input | Bad argument format (e.g., invalid compare arg) |
| 40 | Network/connectivity error | `GerritError::Network` |
| 128 | Git config error | Missing Gerrit host configuration |

These codes are compatible with git-review's exit code conventions.

## Retry Logic

The Gerrit client retries transient errors with exponential backoff:

- **Max retries:** 3 attempts (4 total including the initial request)
- **Backoff schedule:** 1s, 2s, 4s (`1 << attempt`)
- **Logging:** Each retry logs a `warn!` with attempt count, error, and delay
- **Non-retryable errors:** Returned immediately without retry

See `gerrit.rs` `get()` method for implementation details.

## anyhow Context Propagation

All modules use `anyhow::Result<T>` with `.context()` at each call site to build error chains:

```rust
// In app.rs
let config = config::load_config(&repo)
    .context("loading configuration")?;

// In config.rs
let contents = std::fs::read_to_string(&path)
    .context("reading .gitreview")?;

// In gerrit.rs (public API)
let body = self.get(&path).await?;
serde_json::from_str(&body).context("parsing change detail")
```

This produces chained messages like:

```
Error: loading configuration: parsing .gitreview: missing [gerrit] section in .gitreview
```

## CLI Error Display

Errors propagate to `main()`, which uses `{:#}` (anyhow's alternate display) to print the full chain to stderr:

```rust
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {e:#}");
        std::process::exit(exit_code_for_error(&e));
    }
}
```

The `{:#}` format produces `outer: middle: inner` chains. The `{:?}` format (used with `RUST_BACKTRACE=1`) additionally includes a stack backtrace.

## Error Categories

### Network Errors

**Source module:** `gerrit.rs`

| Error | Cause | Exit Code |
|-------|-------|-----------|
| Connection refused | Gerrit server down or wrong host/port | 40 |
| Timeout | Server too slow (>10s connect, >30s request) | 40 |
| DNS resolution failure | Bad hostname | 40 |
| HTTP 401/403 | Bad credentials | 1 |
| HTTP 404 | Bad change ID or deleted change | 1 |
| HTTP 5xx | Server error (retried up to 3 times) | 1 |
| XSSI parse failure | Unexpected response format | 1 |

### Config Errors

**Source modules:** `config.rs`, `app.rs`

| Error | Cause | Exit Code |
|-------|-------|-----------|
| Missing .gitreview | No config file in repo | 1 |
| Missing [gerrit] section | Malformed .gitreview | 1 |
| No host configured | No host in any config source | 128 |
| Bad credentials.toml permissions | File mode > 0600 | 1 |
| TLS enforcement | Credentials over HTTP without --insecure | 1 |

### Git Errors

**Source modules:** `git.rs`, `subprocess.rs`, `hook.rs`

| Error | Cause | Exit Code |
|-------|-------|-----------|
| Not a git repository | Running outside a repo | 1 |
| Detached HEAD | No symbolic ref | 1 |
| git push failure | Remote rejection, auth failure | 1 |
| Hook write failure | Permission denied on hooks dir | 2 |

### User Input Errors

**Source modules:** `push.rs`, `review.rs`, `comments.rs`

| Error | Cause | Exit Code |
|-------|-------|-----------|
| Missing Change-Id | HEAD commit has no trailer | 1 |
| Bad compare argument | Invalid CHANGE,PS format | 3 |
| No change specified | No argument and no Change-Id in HEAD | 1 |

## User-Facing Messages

### Message Quality

Error messages follow two principles:

1. **Chain of causality**: Each `.context()` adds *what was being attempted*, not what went wrong. The innermost error provides the specific failure.
2. **Actionable when possible**: Where the user can fix the problem, the message says how:
   - `Run 'grt setup' to install the commit-msg hook`
   - `Use --insecure to override, or switch to HTTPS`
   - `Fix with: chmod 600 /path/to/credentials.toml`

### Output Destinations

- **Errors**: Always to stderr via `eprintln!()`
- **Data output**: Always to stdout (command results, version info, comment text)
- **Progress messages**: To stderr via `eprintln!()` (e.g., "Downloading change 12345...")
- **Retry warnings**: To stderr via `tracing::warn!`

## Logging

### tracing Integration

grt uses the `tracing` crate for structured logging, separate from error display:

| Flag | Level | Purpose |
|------|-------|---------|
| (none) | `warn` | Warnings only (including retry warnings) |
| `-v` | `info` | Milestone events |
| `-vv` | `debug` | Decision points, paths taken |
| `-vvv` | `trace` | Data-level detail |

## Future Evolution

### Additional Typed Error Enums

Candidates for future typed enums:

- `ConfigError` -- distinguish missing file from parse error from validation error
- `GitError` -- distinguish "not a repo" from "detached HEAD" from subprocess failures

### TUI Error Handling

The TUI (planned post-MVP) will require non-fatal error handling. Errors must not crash the application -- they should be displayed in a status bar.

### miette Integration

The architecture doc describes potential `miette` integration for rich error rendering with source spans, particularly useful for config file parse errors.

## Divergences from Architecture Doc

| Architecture doc describes | Current status |
|---------------------------|---------------|
| `thiserror` enums at module boundaries | Implemented for `GerritError` only |
| `GerritError` type | Implemented with 4 variants |
| `DbError`, `SearchError` types | Not needed yet (no DB or search) |
| TUI error handling (status bar display) | No TUI yet |
| Retry with exponential backoff | Implemented (3 retries, 1s/2s/4s) |
| Structured exit codes | Implemented (0/1/2/3/40/128) |
| `ConfigError` with miette span information | Not implemented |
| `SyncEvent::Error` channel pattern | No sync engine yet |
