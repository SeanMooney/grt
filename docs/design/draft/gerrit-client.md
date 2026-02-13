# grt Gerrit Client

**Related ref-specs:** `../ref-specs/git-review-gerrit-api.md`, `../ref-specs/gertty-sync-system.md`
**Source code:** `crates/grt/src/gerrit.rs`
**Status:** Draft

## Overview

grt communicates with Gerrit exclusively through its REST API over HTTP/HTTPS. There is no SSH protocol support -- the REST API is strictly more capable than Gerrit's SSH command interface, and a single protocol path simplifies the client significantly.

The client is built on `reqwest`, an async HTTP client for Rust. It handles Gerrit's XSSI prefix stripping, HTTP Basic authentication with manual base64 encoding, response deserialization via serde, and connection management with configurable timeouts.

The `GerritClient` struct owns a `reqwest::Client` instance (which provides connection pooling internally) along with the base URL and optional credentials. All public methods are `async` and return `anyhow::Result<T>`.

## Endpoints

grt implements six Gerrit REST API endpoints, all read-only. Write operations (submitting reviews, setting topics) are deferred to post-MVP.

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

Queries changes using [Gerrit's query syntax](https://gerrit-review.googlesource.com/Documentation/user-search.html). The query string is URL-encoded. Always requests `CURRENT_REVISION` and `DETAILED_ACCOUNTS` options to include revision details and full account info in the response.

### `get_change_detail` -- Full Change Info

```
GET /a/changes/{id}/detail?o=CURRENT_REVISION&o=DETAILED_ACCOUNTS&o=MESSAGES
```

Returns detailed information for a single change, including review messages. The `{id}` parameter is URL-encoded and can be a change number, Change-Id, or triplet (`project~branch~Change-Id`). Used by `grt comments` to fetch the change metadata and message history.

### `get_change_comments` / `get_revision_comments` -- Inline Comments

```
GET /a/changes/{id}/comments
GET /a/changes/{id}/revisions/{revision}/comments
```

Returns inline comments organized by file path. `get_change_comments` returns comments across all revisions; `get_revision_comments` scopes to a specific patchset. The response type is `HashMap<String, Vec<CommentInfo>>` where keys are file paths.

### `get_robot_comments` -- Automated Comments

```
GET /a/changes/{id}/robotcomments
```

Returns automated/CI comments in the same format as human comments. Used when `--include-robot-comments` is passed to `grt comments`.

### Endpoints Not Yet Implemented

The following endpoints from the stub are not implemented in the MVP:

- **Submit Review** (`POST /changes/{id}/revisions/{rev}/review`) -- planned for TUI mode
- **Set Topic/Hashtags** -- planned for future write operations
- **Cherry-pick** (`POST /changes/{id}/revisions/{rev}/cherrypick`) -- planned post-MVP

## Authentication

### HTTP Basic Auth

Authentication uses HTTP Basic with manual base64 encoding. When credentials are present, the client:

1. Adds the `/a/` prefix to all endpoint paths (Gerrit's convention for authenticated access)
2. Constructs an `Authorization: Basic <base64(username:password)>` header

```rust
fn auth_headers(&self) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(ref creds) = self.credentials {
        let encoded = base64_encode(&format!("{}:{}", creds.username, creds.password));
        if let Ok(val) = HeaderValue::from_str(&format!("Basic {encoded}")) {
            headers.insert(AUTHORIZATION, val);
        }
    }
    headers
}
```

### Manual Base64 Encoder

Rather than adding a dependency for base64 encoding (which is the only use case), grt includes a minimal `Base64Encoder` (~60 lines) that implements `std::io::Write`. It processes input in 3-byte blocks, producing 4 base64 characters per block with `=` padding for the final block.

### Credential Redaction

The `Credentials` struct has a custom `Debug` implementation that redacts the password field, preventing accidental credential exposure in debug logs:

```rust
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}
```

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

The `_account_id` field uses Gerrit's underscore-prefixed convention for internal fields, mapped via `#[serde(rename)]`.

### ChangeInfo

```rust
pub struct ChangeInfo {
    pub id: Option<String>,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub change_id: Option<String>,
    pub subject: Option<String>,
    pub status: Option<String>,
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

The `revisions` field is a map from commit SHA to `RevisionInfo`, present only when `CURRENT_REVISION` or `ALL_REVISIONS` is requested.

### RevisionInfo and CommitInfo

```rust
pub struct RevisionInfo {
    #[serde(rename = "_number")]
    pub number: Option<i32>,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub commit: Option<CommitInfo>,
}

pub struct CommitInfo {
    pub subject: Option<String>,
    pub message: Option<String>,
    pub author: Option<GitPersonInfo>,
    pub committer: Option<GitPersonInfo>,
}
```

The `ref` field requires renaming since `ref` is a Rust keyword.

### CommentInfo

```rust
pub struct CommentInfo {
    pub id: Option<String>,
    pub path: Option<String>,
    pub line: Option<i32>,
    pub range: Option<CommentRange>,
    pub in_reply_to: Option<String>,
    pub message: Option<String>,
    pub updated: Option<String>,
    pub author: Option<AccountInfo>,
    pub patch_set: Option<i32>,
    pub unresolved: Option<bool>,
}
```

The `in_reply_to` field links to the parent comment's `id`, forming reply chains that the `comments` module resolves into threaded conversations.

### ChangeMessageInfo

```rust
pub struct ChangeMessageInfo {
    pub id: Option<String>,
    pub author: Option<AccountInfo>,
    pub date: Option<String>,
    pub message: Option<String>,
    #[serde(rename = "_revision_number")]
    pub revision_number: Option<i32>,
}
```

## Error Handling

### HTTP Status Errors

Non-2xx responses are treated as errors. The client reads the response body (which often contains a human-readable Gerrit error message) and includes it in the error:

```rust
if !status.is_success() {
    let body = resp.text().await.unwrap_or_default();
    anyhow::bail!("Gerrit API error ({}): {}", status, body);
}
```

### Response Parsing

JSON deserialization errors are wrapped with context describing which operation failed:

```rust
serde_json::from_str(&body).context("parsing change detail")
```

### XSSI Prefix Stripping

Gerrit prepends `)]}'\n` (or `)]}\n` without the quote) to JSON responses as an XSSI prevention measure. The `strip_xssi_prefix` function handles this dynamically rather than hardcoding a character count:

```rust
pub fn strip_xssi_prefix(body: &str) -> String {
    if let Some(newline_pos) = body.find('\n') {
        let prefix = &body[..newline_pos];
        if prefix.starts_with(")]}") {
            return body[newline_pos + 1..].to_string();
        }
    }
    body.to_string()
}
```

This approach is more robust than git-review's `request.text[4:]` which hardcodes 4 characters. It handles variations in the prefix (with or without the trailing quote) and passes through responses that lack a prefix entirely.

### No Retry Logic

The MVP client does not implement retry or exponential backoff. Failed requests return an error immediately. Retry logic is planned for post-MVP, particularly for the sync engine where transient 5xx errors and connection resets should be retried.

## Connection Management

### Client Configuration

The `reqwest::Client` is configured at construction time with:

| Setting | Value | Rationale |
|---------|-------|-----------|
| Connect timeout | 10 seconds | Fail fast on unreachable hosts |
| Request timeout | 30 seconds | Bound total request duration including response body |
| User-Agent | `grt/{version}` | Identifies the client in Gerrit access logs |

```rust
let client = reqwest::Client::builder()
    .connect_timeout(CONNECT_TIMEOUT)
    .timeout(REQUEST_TIMEOUT)
    .user_agent(format!("grt/{}", env!("CARGO_PKG_VERSION")))
    .build()
    .context("building HTTP client")?;
```

### Connection Pooling

`reqwest::Client` internally maintains a connection pool with keep-alive. Multiple requests to the same Gerrit server reuse TCP connections, reducing latency for sequences of API calls (e.g., fetching change detail then comments).

### URL Construction

The `api_url` method constructs full API URLs by joining the endpoint path onto the base URL. When credentials are present, the `/a` prefix is automatically prepended for Gerrit's authenticated endpoints:

```rust
fn api_url(&self, path: &str) -> Result<Url> {
    let prefix = if self.credentials.is_some() { "/a" } else { "" };
    let full_path = format!("{}{}", prefix, path);
    self.base_url.join(&full_path).context("constructing API URL")
}
```

## Divergences from Ref-Specs

### vs. git-review (`git-review-gerrit-api.md`)

- **REST-only**: git-review supports both SSH and HTTP protocols, choosing based on the remote URL scheme. grt uses REST exclusively -- SSH adds no API capabilities that REST lacks.
- **No 401-retry**: git-review makes an unauthenticated request first, then retries with credentials on 401. grt sends credentials upfront when available, avoiding the round-trip.
- **Dynamic XSSI stripping**: git-review hardcodes `text[4:]`. grt finds the first newline and checks for the `)]}` prefix, handling format variations gracefully.
- **No response normalization**: git-review translates HTTP response format to match SSH output. grt works directly with the REST API's native JSON structure.
- **Manual base64**: git-review uses Python's standard library base64. grt includes a minimal encoder to avoid adding a crate dependency for a single use case.
- **Timeouts**: git-review sets no timeouts. grt configures both connect (10s) and request (30s) timeouts.

### vs. gertty (`gertty-sync-system.md`)

- **No sync engine**: gertty's `GerritClient` is deeply integrated with its sync task system (27 task types, priority queue, offline handling). grt's client is a standalone HTTP layer with no sync logic -- sync will be added post-MVP.
- **No retry/backoff**: gertty retries on connection errors with a 30-second sleep. grt does not retry in the MVP.
- **No version-gated features**: gertty checks the server version to gate features (robot comments require >= 2.14.0). grt does not perform version checks.
- **Narrower endpoint surface**: gertty uses dozens of endpoints (projects, branches, labels, checks, edits). grt uses six read-only endpoints.
