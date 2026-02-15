# grt Gerrit Client

**Related ref-specs:** `../ref-specs/git-review-gerrit-api.md`, `../ref-specs/gertty-sync-system.md`
**Source code:** `crates/grt/src/gerrit.rs`
**Status:** Draft

## Overview

grt communicates with Gerrit through either its REST API (HTTP/HTTPS) or SSH, depending on the remote URL. When the remote uses `http://` or `https://`, grt uses the REST API. When the remote uses `ssh://` or SCP-style URLs, grt uses `ssh ... gerrit query` for change queries (list, download, cherry-pick, compare), matching git-review's dual-protocol behavior.

The REST client is built on `reqwest`, an async HTTP client for Rust. It handles Gerrit's XSSI prefix stripping, HTTP Basic and Bearer token authentication, response deserialization via serde, connection management with configurable timeouts, and retry with exponential backoff on transient failures.

The `GerritClient` struct owns a `reqwest::Client` instance (which provides connection pooling internally) along with the base URL and optional credentials. All public methods are `async` and return `anyhow::Result<T>`.

## Endpoints

grt implements seven Gerrit REST API endpoints, all read-only. Write operations (submitting reviews, setting topics) are deferred to post-MVP.

### `get_version` -- Server Version

```
GET /config/server/version
```

Returns the Gerrit server version as a quoted JSON string (e.g., `"3.9.1"`). Used by `grt version` and `grt setup` to verify connectivity. This endpoint does not require authentication, making it useful as a lightweight connectivity probe.

### `get_self_account` -- Authenticated User

```
GET /a/accounts/self
```

Returns the authenticated user's `AccountInfo`. Used by `grt setup` to verify credentials and by `grt comments` (via `authenticate_and_verify`) to confirm auth before fetching comments. Requires authentication -- the `/a/` prefix is added automatically when credentials are present.

### `query_changes` -- Search Changes

```
GET /a/changes/?q={query}&o=CURRENT_REVISION&o=DETAILED_ACCOUNTS
```

Queries changes using Gerrit's query syntax. Used by `grt review -l` (list mode) to find open changes for the current project.

### `get_change_detail` -- Full Change Info

```
GET /a/changes/{id}/detail?o=CURRENT_REVISION&o=DETAILED_ACCOUNTS&o=MESSAGES
```

Returns detailed information for a single change, including review messages. Used by `grt comments` to fetch the change metadata and message history.

### `get_change_all_revisions` -- Change with All Patchsets

```
GET /a/changes/{id}/detail?o=ALL_REVISIONS&o=DETAILED_ACCOUNTS
```

Returns change detail with all revision data. Used by download (`-d`), cherry-pick (`-x`, `-X`, `-N`), and compare (`-m`) modes to access specific patchset refs.

### `get_change_comments` / `get_revision_comments` -- Inline Comments

```
GET /a/changes/{id}/comments
GET /a/changes/{id}/revisions/{revision}/comments
```

Returns inline comments organized by file path.

### `get_robot_comments` -- Automated Comments

```
GET /a/changes/{id}/robotcomments
```

Returns automated/CI comments in the same format as human comments.

### Endpoints Not Yet Implemented

- **Submit Review** (`POST /changes/{id}/revisions/{rev}/review`) -- planned for TUI mode
- **Set Topic/Hashtags** -- planned for future write operations

## Authentication

### HTTP Basic Auth (Default)

Authentication uses HTTP Basic with manual base64 encoding. When credentials are present, the client:

1. Adds the `/a/` prefix to all endpoint paths (Gerrit's convention for authenticated access)
2. Constructs an `Authorization: Basic <base64(username:password)>` header

### Bearer Token Auth

When `auth_type = "bearer"` is configured in `credentials.toml`, the client uses Bearer token authentication:

1. Adds the `/a/` prefix (same as Basic)
2. Constructs an `Authorization: Bearer <token>` header, using the password field as the token

```rust
fn auth_headers(&self) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(ref creds) = self.credentials {
        let header_value = match creds.auth_type {
            AuthType::Bearer => format!("Bearer {}", creds.password),
            AuthType::Basic => {
                let encoded = base64_encode(&format!("{}:{}", creds.username, creds.password));
                format!("Basic {encoded}")
            }
        };
        if let Ok(val) = HeaderValue::from_str(&header_value) {
            headers.insert(AUTHORIZATION, val);
        }
    }
    headers
}
```

### Manual Base64 Encoder

Rather than adding a dependency for base64 encoding (which is the only use case), grt includes a minimal `Base64Encoder` (~60 lines) that implements `std::io::Write`. It processes input in 3-byte blocks, producing 4 base64 characters per block with `=` padding for the final block.

### Credential Redaction

The `Credentials` struct has a custom `Debug` implementation that redacts the password field, preventing accidental credential exposure in debug logs.

### TLS Enforcement

Credential transmission over plain HTTP is blocked by default. The `App::authenticate()` method refuses to send credentials unless the scheme is `https` or the `--insecure` flag is set. This check happens at the application layer (in `app.rs`), not in the Gerrit client itself.

## Request/Response Types

All response types use `serde::Deserialize` with liberal use of `Option<T>` fields, since Gerrit omits fields that have no value rather than returning null.

### AccountInfo

```rust
pub struct AccountInfo {
    #[serde(rename = "_account_id")]
    pub account_id: i64,
    pub name: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
}
```

### ChangeInfo

```rust
pub struct ChangeInfo {
    pub id: Option<String>,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub change_id: Option<String>,
    pub subject: Option<String>,
    pub status: Option<String>,
    pub topic: Option<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    #[serde(rename = "_number")]
    pub number: Option<i64>,
    pub owner: Option<AccountInfo>,
    pub current_revision: Option<String>,
    pub revisions: Option<HashMap<String, RevisionInfo>>,
    pub messages: Option<Vec<ChangeMessageInfo>>,
    pub insertions: Option<i64>,
    pub deletions: Option<i64>,
}
```

The `revisions` field is a map from commit SHA to `RevisionInfo`, present only when `CURRENT_REVISION` or `ALL_REVISIONS` is requested. The `topic` field is used by list mode's verbose output.

### RevisionInfo and CommitInfo

```rust
pub struct RevisionInfo {
    #[serde(rename = "_number")]
    pub number: Option<i32>,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub commit: Option<CommitInfo>,
}
```

The `ref` field (renamed as `git_ref` since `ref` is a Rust keyword) provides the fetch ref (e.g., `refs/changes/45/12345/1`) used by download and cherry-pick modes.

### CommentInfo and ChangeMessageInfo

Standard Gerrit comment types used by the comments command. See `gerrit.rs` for full definitions.

## Error Handling

### Typed Error Enum

The `GerritError` enum provides typed errors for the Gerrit client:

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

Each variant classifies the failure type, enabling callers to match on specific error conditions (e.g., for exit code mapping).

### Retry with Exponential Backoff

The internal `get()` method wraps `get_once()` with automatic retry:

- **Retryable errors:** 5xx server errors and network failures (`GerritError::is_retryable()`)
- **Non-retryable errors:** 401/403 auth failures, 404 not found, 4xx client errors
- **Max retries:** 3 attempts
- **Backoff:** 1s, 2s, 4s (exponential, `1 << attempt`)
- **Logging:** Each retry logs a warning with attempt count and delay

```rust
async fn get(&self, path: &str) -> Result<String> {
    let url = self.api_url(path)?;
    let mut last_err = None;

    for attempt in 0..=MAX_RETRIES {
        match self.get_once(&url).await {
            Ok(body) => return Ok(body),
            Err(e) if e.is_retryable() && attempt < MAX_RETRIES => {
                let delay = Duration::from_secs(1 << attempt);
                warn!("request to {} failed (attempt {}/{}): {}, retrying in {}s",
                    path, attempt + 1, MAX_RETRIES + 1, e, delay.as_secs());
                tokio::time::sleep(delay).await;
                last_err = Some(e);
            }
            Err(e) => {
                return Err(e).context(format!("Gerrit API request to {path}"));
            }
        }
    }

    Err(last_err.unwrap()).context(format!("Gerrit API request to {path} (exhausted retries)"))
}
```

### XSSI Prefix Stripping

Gerrit prepends `)]}'\n` (or `)]}\n` without the quote) to JSON responses as an XSSI prevention measure. The `strip_xssi_prefix` function handles this dynamically by finding the first newline and checking for the `)]}` prefix. This is more robust than git-review's hardcoded `text[4:]`.

## Connection Management

### Client Configuration

| Setting | Value | Rationale |
|---------|-------|-----------|
| Connect timeout | 10 seconds | Fail fast on unreachable hosts |
| Request timeout | 30 seconds | Bound total request duration including response body |
| User-Agent | `grt/{version}` | Identifies the client in Gerrit access logs |

### Connection Pooling

`reqwest::Client` internally maintains a connection pool with keep-alive. Multiple requests to the same Gerrit server reuse TCP connections.

## Divergences from Ref-Specs

### vs. git-review (`git-review-gerrit-api.md`)

- **Dual protocol**: grt matches git-review: HTTP/HTTPS remotes use REST; SSH/SCP remotes use `ssh gerrit query` for change metadata. Transport is selected from the resolved remote URL (pushurl + insteadOf/pushInsteadOf).
- **No 401-retry**: git-review makes an unauthenticated request first, then retries with credentials on 401. grt sends credentials upfront when available.
- **Dynamic XSSI stripping**: git-review hardcodes `text[4:]`. grt finds the first newline and checks for the `)]}` prefix.
- **Retry with backoff**: git-review does not retry on transient errors. grt retries 5xx and network failures with exponential backoff.
- **Bearer auth support**: git-review supports basic auth only. grt supports both Basic and Bearer token authentication.
- **Typed errors**: git-review uses generic exceptions. grt uses a `GerritError` enum that classifies failures by type, enabling retryability checks and exit code mapping.
- **Timeouts**: git-review sets no timeouts. grt configures both connect (10s) and request (30s) timeouts.

### vs. gertty (`gertty-sync-system.md`)

- **No sync engine**: gertty's `GerritClient` is deeply integrated with its sync task system. grt's client is a standalone HTTP layer.
- **Structured retry**: gertty retries with a fixed 30-second sleep. grt uses exponential backoff with a retryability classifier.
- **No version-gated features**: gertty checks the server version to gate features. grt does not perform version checks.
- **Narrower endpoint surface**: gertty uses dozens of endpoints. grt uses seven read-only endpoints.
