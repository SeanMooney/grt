# grt Error Handling

**Related docs:** `architecture.md`, `../adopted/tech-stack.md`
**Source code:** All modules
**Status:** Draft

## Overview

grt uses `anyhow` as its sole error type throughout the MVP. Every public function returns `anyhow::Result<T>`. Context is added via `.context("description")` and `.with_context(|| ...)` at each layer of the call stack, building a chain of causality that produces readable error messages for the user.

There are no typed error enums in the MVP. The architecture doc describes a future two-tier strategy with `thiserror` enums at module boundaries, but the current implementation uses `anyhow` exclusively.

## Current Error Strategy

### anyhow Throughout

Every module uses `anyhow::Result<T>`:

```rust
// gerrit.rs
pub async fn get_change_detail(&self, change_id: &str) -> Result<ChangeInfo> { ... }

// config.rs
pub fn load_config(repo: &GitRepo) -> Result<GerritConfig> { ... }

// git.rs
pub fn current_branch(&self) -> Result<String> { ... }

// push.rs
pub fn build_refspec(opts: &PushOptions) -> Result<String> { ... }
```

### Context Propagation

Context is added at each call site to build a chain. The `?` operator propagates errors up the call stack, and `.context()` wraps each layer:

```rust
// In app.rs
let config = config::load_config(&repo)
    .context("loading configuration")?;

// In config.rs
let contents = std::fs::read_to_string(&path)
    .context("reading .gitreview")?;
let parsed = parse_gitreview(&contents)
    .context("parsing .gitreview")?;

// In gerrit.rs
let resp = self.client.get(url)
    .headers(self.auth_headers())
    .send()
    .await
    .context("sending request to Gerrit")?;
```

This produces chained messages like:

```
Error: loading configuration: parsing .gitreview: missing [gerrit] section in .gitreview
```

### CLI Error Display

Errors propagate to `main()`, which uses `{:#}` (anyhow's alternate display) to print the full chain:

```rust
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {:#}", e);
        std::process::exit(1);
    }
}
```

The `{:#}` format produces `outer: middle: inner` chains. The `{:?}` format (used with `RUST_BACKTRACE=1`) additionally includes a stack backtrace.

### No Recovery Logic

The MVP has no retry or recovery paths. All errors are terminal -- they propagate to `main()` and exit with code 1. The only exception is the version command, which catches Gerrit connectivity errors and prints "unavailable" instead of failing:

```rust
match client.get_version().await {
    Ok(version) => println!("Gerrit {}", version),
    Err(_) => println!("Gerrit unavailable"),
}
```

## Error Categories

### Network Errors

**Source module:** `gerrit.rs`

| Error | Cause | User sees |
|-------|-------|-----------|
| Connection refused | Gerrit server down or wrong host/port | `sending request to Gerrit: error sending request: connection refused` |
| Timeout | Server too slow (>10s connect, >30s request) | `sending request to Gerrit: request timeout` |
| DNS resolution failure | Bad hostname | `sending request to Gerrit: error sending request: dns error` |
| HTTP 401 Unauthorized | Bad credentials | `Gerrit API error (401 Unauthorized): Unauthorized` |
| HTTP 404 Not Found | Bad change ID or deleted change | `Gerrit API error (404 Not Found): Not found: ...` |
| HTTP 5xx | Server error | `Gerrit API error (500 Internal Server Error): ...` |
| XSSI parse failure | Unexpected response format | `parsing change detail: expected value at line 1 column 1` |

### Config Errors

**Source modules:** `config.rs`, `app.rs`

| Error | Cause | User sees |
|-------|-------|-----------|
| Missing .gitreview | No config file in repo | `reading .gitreview: No such file or directory` |
| Missing [gerrit] section | Malformed .gitreview | `parsing .gitreview: missing [gerrit] section in .gitreview` |
| Invalid port | Non-numeric port in .gitreview | `parsing .gitreview: parsing port in .gitreview: invalid digit` |
| No host configured | No host in any config source | `no Gerrit host configured. Create a .gitreview file or set gitreview.host in git config` |
| Bad credentials.toml permissions | File mode > 0600 | `credentials.toml has permissions 0644, expected 0600` |
| Credential TOML parse error | Invalid TOML syntax | `parsing credentials.toml: TOML parse error at line ...` |
| TLS enforcement | Trying to send credentials over HTTP | `refusing to send credentials over plain HTTP (scheme: http). Use --insecure to override` |

### Git Errors

**Source modules:** `git.rs`, `subprocess.rs`, `hook.rs`

| Error | Cause | User sees |
|-------|-------|-----------|
| Not a git repository | Running outside a repo | `discovering git repository: could not find repository` |
| Detached HEAD | No symbolic ref | `HEAD is detached` |
| Bare repository | No worktree | `repository is bare (no worktree)` |
| git push failure | Remote rejection, auth failure | `git push failed (exit 128): ...` (with Gerrit's error message) |
| Hook write failure | Permission denied on hooks dir | `writing commit-msg hook: Permission denied` |
| Missing remote | Remote not configured | `git remote get-url failed: ...` |
| git credential failure | No credential helper configured | `git credential fill failed: ...` |

### User Input Errors

**Source modules:** `push.rs`, `comments.rs`

| Error | Cause | User sees |
|-------|-------|-----------|
| Missing Change-Id | HEAD commit has no trailer | `HEAD commit is missing a Change-Id trailer. Run 'grt setup' to install the commit-msg hook, then amend the commit` |
| Invalid Change-Id | Trailer doesn't match format | Same as above (validation rejects malformed IDs) |
| No change specified | No argument and no Change-Id in HEAD | `no change specified and could not extract Change-Id from HEAD commit` |
| No unpushed commits | Nothing to push | `No unpushed commits found.` (not an error -- exits 0) |
| Whitespace in reviewer | Invalid reviewer name | Rejected during refspec construction |

## User-Facing Messages

### Message Quality

Error messages follow two principles:

1. **Chain of causality**: Each `.context()` adds *what was being attempted*, not what went wrong. The innermost error provides the specific failure.

   Good: `loading configuration: parsing .gitreview: invalid digit found in string`
   Bad: `config error: parse error: error`

2. **Actionable when possible**: Where the user can fix the problem, the message says how:
   - `Run 'grt setup' to install the commit-msg hook`
   - `Use --insecure to override, or switch to HTTPS`
   - `Fix with: chmod 600 /path/to/credentials.toml`

### Output Destinations

- **Errors**: Always to stderr via `eprintln!()`
- **Data output**: Always to stdout (command results, version info, comment text)
- **Progress messages**: To stderr via `eprintln!()` (e.g., setup step output)

This separation allows scripting: `grt comments --format json 2>/dev/null` gives clean JSON on stdout.

## Logging

### tracing Integration

grt uses the `tracing` crate for structured logging, separate from error display:

```rust
tracing::debug!("loading config from {:?}", path);
tracing::info!("connected to Gerrit {}", version);
tracing::warn!("credentials.toml not found, trying git credential helper");
```

Log output goes to stderr via `tracing-subscriber` with a configurable level controlled by `-v` flags:

| Flag | Level | Purpose |
|------|-------|---------|
| (none) | `warn` | Warnings only |
| `-v` | `info` | Milestone events |
| `-vv` | `debug` | Decision points, paths taken |
| `-vvv` | `trace` | Data-level detail |

Tracing is configured without timestamps and without the target module path, keeping output compact for CLI use:

```
 DEBUG loading config from "/home/user/project/.gitreview"
  INFO connected to Gerrit 3.9.1
```

### Separation from Error Display

Tracing and error display serve different audiences:

- **Tracing** is for debugging grt itself (developers, bug reporters). Controlled by `-v`.
- **Error messages** are for end users. Always displayed on failure.

A typical error flow:

```
DEBUG attempting to read .gitreview       <- tracing (only with -v)
DEBUG .gitreview not found, trying git config  <- tracing
Error: no Gerrit host configured          <- error (always shown)
```

## Future Evolution

### Typed Error Enums

The architecture doc (`architecture.md`) describes a future two-tier error strategy where module boundaries use `thiserror` enums:

```rust
// Planned, not yet implemented
#[derive(Debug, thiserror::Error)]
pub enum GerritError {
    #[error("authentication failed")]
    AuthFailed,
    #[error("change not found: {0}")]
    NotFound(String),
    #[error("server error ({0})")]
    ServerError(u16),
    #[error("network error")]
    NetworkError(#[source] reqwest::Error),
}
```

This would enable callers to match on specific error types:

```rust
match client.get_change(id).await {
    Err(e) if e.downcast_ref::<GerritError>() == Some(&GerritError::AuthFailed) => {
        // Handle auth failure specifically
    }
    Err(e) => return Err(e),
    Ok(change) => { ... }
}
```

**Candidates for typed enums:**

- `GerritError` -- distinguish retryable (network, 5xx) from permanent (auth, 404) failures
- `ConfigError` -- distinguish missing file from parse error from validation error
- `GitError` -- distinguish "not a repo" from "detached HEAD" from subprocess failures

### TUI Error Handling

The TUI (planned post-MVP) will require non-fatal error handling. Errors must not crash the application:

```rust
// Planned pattern from architecture.md
match app.fetch_comments(id).await {
    Ok(comments) => self.display_comments(comments),
    Err(e) => self.status_message = format!("{:#}", e),
}
```

Background sync errors would flow through a channel as `SyncEvent::Error(String)` events.

### Retry Logic

Planned for the sync engine. Transient errors (connection reset, 5xx, timeout) should be retried with exponential backoff. The typed `GerritError` enum would enable distinguishing retryable from permanent failures.

## Divergences from Architecture Doc

The architecture doc describes several error handling features not yet implemented:

| Architecture doc describes | MVP status |
|---------------------------|------------|
| `thiserror` enums at module boundaries | Not implemented -- `anyhow` throughout |
| `GerritError`, `DbError`, `SearchError` types | Not implemented |
| TUI error handling (status bar display) | No TUI yet |
| Retry with exponential backoff | No retry logic |
| `ConfigError` with miette span information | No miette integration |
| Structured exit codes (2 for auth) | Exit code 1 for all errors |
| `SyncEvent::Error` channel pattern | No sync engine yet |

These are planned for implementation as the corresponding features are built.
