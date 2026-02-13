// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::Deserialize;
use tracing::warn;
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RETRIES: u32 = 3;

/// Typed errors from the Gerrit REST API.
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

impl GerritError {
    /// Whether this error is transient and worth retrying.
    pub fn is_retryable(&self) -> bool {
        match self {
            GerritError::ServerError { status, .. } => *status >= 500,
            GerritError::Network(_) => true,
            _ => false,
        }
    }
}

/// Type of HTTP authentication to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthType {
    /// HTTP Basic authentication (default).
    #[default]
    Basic,
    /// Bearer token authentication.
    Bearer,
}

/// Credentials for HTTP authentication.
#[derive(Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub auth_type: AuthType,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// Client for the Gerrit REST API.
#[derive(Debug)]
pub struct GerritClient {
    client: reqwest::Client,
    base_url: Url,
    credentials: Option<Credentials>,
}

impl GerritClient {
    /// Create a new Gerrit REST client.
    ///
    /// When `ssl_verify` is `false`, TLS certificate verification is disabled.
    pub fn new(base_url: Url, credentials: Option<Credentials>, ssl_verify: bool) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent(format!("grt/{}", env!("CARGO_PKG_VERSION")));

        if !ssl_verify {
            builder = builder.danger_accept_invalid_certs(true);
        }

        let client = builder.build().context("building HTTP client")?;

        Ok(Self {
            client,
            base_url,
            credentials,
        })
    }

    /// Set or replace the credentials used for authentication.
    pub fn set_credentials(&mut self, creds: Credentials) {
        self.credentials = Some(creds);
    }

    /// Return the current credentials, if any.
    pub fn credentials(&self) -> Option<&Credentials> {
        self.credentials.as_ref()
    }

    /// Build the full URL for an API endpoint path.
    ///
    /// Appends to the base URL's existing path instead of using `Url::join`,
    /// which would discard any sub-path prefix (e.g. `/gerrit/`).
    fn api_url(&self, path: &str) -> Result<Url> {
        // Gerrit authenticated endpoints use /a/ prefix
        let prefix = if self.credentials.is_some() { "/a" } else { "" };
        let full_path = format!("{}{}", prefix, path);

        // Split off any query string so set_path doesn't percent-encode `?`.
        let (path_part, query_part) = match full_path.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (full_path.as_str(), None),
        };

        let mut url = self.base_url.clone();
        {
            let base_path = url.path().trim_end_matches('/');
            let new_path = format!("{}{}", base_path, path_part);
            url.set_path(&new_path);
        }
        url.set_query(query_part);
        Ok(url)
    }

    /// Build authorization headers if credentials are available.
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

    /// Perform a single GET request returning a typed error.
    async fn get_once(&self, url: &Url) -> std::result::Result<String, GerritError> {
        let resp = self
            .client
            .get(url.clone())
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| GerritError::Network(e.to_string()))?;

        let status = resp.status().as_u16();
        if status == 401 || status == 403 {
            return Err(GerritError::AuthFailed { status });
        }
        if status == 404 {
            return Err(GerritError::NotFound);
        }
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GerritError::ServerError { status, body });
        }

        let body = resp
            .text()
            .await
            .map_err(|e| GerritError::Network(e.to_string()))?;
        Ok(strip_xssi_prefix(&body))
    }

    /// Perform a GET request with retry on transient errors.
    ///
    /// Retries up to 3 times with exponential backoff (1s, 2s, 4s) on
    /// 5xx server errors and network failures. Does not retry on 4xx.
    async fn get(&self, path: &str) -> Result<String> {
        let url = self.api_url(path)?;
        let mut last_err = None;

        for attempt in 0..=MAX_RETRIES {
            match self.get_once(&url).await {
                Ok(body) => return Ok(body),
                Err(e) if e.is_retryable() && attempt < MAX_RETRIES => {
                    let delay = Duration::from_secs(1 << attempt);
                    warn!(
                        "request to {} failed (attempt {}/{}): {}, retrying in {}s",
                        path,
                        attempt + 1,
                        MAX_RETRIES + 1,
                        e,
                        delay.as_secs()
                    );
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

    /// Get the Gerrit server version.
    pub async fn get_version(&self) -> Result<String> {
        let body = self.get("/config/server/version").await?;
        let version: String = serde_json::from_str(&body).context("parsing server version")?;
        Ok(version)
    }

    /// Get the authenticated user's account info.
    pub async fn get_self_account(&self) -> Result<AccountInfo> {
        let body = self.get("/accounts/self").await?;
        serde_json::from_str(&body).context("parsing account info")
    }

    /// Query changes using Gerrit query syntax.
    pub async fn query_changes(&self, query: &str) -> Result<Vec<ChangeInfo>> {
        let encoded_query = urlencoding::encode(query);
        let path = format!(
            "/changes/?q={}&o=CURRENT_REVISION&o=DETAILED_ACCOUNTS",
            encoded_query
        );
        let body = self.get(&path).await?;
        serde_json::from_str(&body).context("parsing change list")
    }

    /// Get detailed change information.
    pub async fn get_change_detail(&self, change_id: &str) -> Result<ChangeInfo> {
        let path = format!(
            "/changes/{}/detail?o=CURRENT_REVISION&o=DETAILED_ACCOUNTS&o=MESSAGES",
            urlencoding::encode(change_id)
        );
        let body = self.get(&path).await?;
        serde_json::from_str(&body).context("parsing change detail")
    }

    /// Get change detail with ALL_REVISIONS (needed for download/cherry-pick).
    pub async fn get_change_all_revisions(&self, change_id: &str) -> Result<ChangeInfo> {
        let path = format!(
            "/changes/{}/detail?o=ALL_REVISIONS&o=DETAILED_ACCOUNTS",
            urlencoding::encode(change_id)
        );
        let body = self.get(&path).await?;
        serde_json::from_str(&body).context("parsing change detail with all revisions")
    }

    /// Get all comments on a change (all revisions).
    pub async fn get_change_comments(
        &self,
        change_id: &str,
    ) -> Result<HashMap<String, Vec<CommentInfo>>> {
        let path = format!("/changes/{}/comments", urlencoding::encode(change_id));
        let body = self.get(&path).await?;
        serde_json::from_str(&body).context("parsing change comments")
    }

    /// Get comments on a specific revision.
    pub async fn get_revision_comments(
        &self,
        change_id: &str,
        revision: &str,
    ) -> Result<HashMap<String, Vec<CommentInfo>>> {
        let path = format!(
            "/changes/{}/revisions/{}/comments",
            urlencoding::encode(change_id),
            urlencoding::encode(revision)
        );
        let body = self.get(&path).await?;
        serde_json::from_str(&body).context("parsing revision comments")
    }

    /// Get robot comments on a change.
    pub async fn get_robot_comments(
        &self,
        change_id: &str,
    ) -> Result<HashMap<String, Vec<CommentInfo>>> {
        let path = format!("/changes/{}/robotcomments", urlencoding::encode(change_id));
        let body = self.get(&path).await?;
        serde_json::from_str(&body).context("parsing robot comments")
    }
}

/// Strip the XSSI prevention prefix from Gerrit API responses.
/// Gerrit prepends `)]}'` (with or without the closing quote) followed by a newline.
pub fn strip_xssi_prefix(body: &str) -> String {
    if let Some(newline_pos) = body.find('\n') {
        let prefix = &body[..newline_pos];
        if prefix.starts_with(")]}") {
            return body[newline_pos + 1..].to_string();
        }
    }
    body.to_string()
}

fn base64_encode(input: &str) -> String {
    use std::io::Write;
    let mut buf = Vec::new();
    {
        let mut encoder = Base64Encoder::new(&mut buf);
        encoder.write_all(input.as_bytes()).unwrap();
        encoder.finish().unwrap();
    }
    String::from_utf8(buf).unwrap()
}

/// Minimal base64 encoder (avoids adding a dependency just for this).
struct Base64Encoder<W: std::io::Write> {
    writer: W,
    buf: [u8; 3],
    buf_len: usize,
}

const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

impl<W: std::io::Write> Base64Encoder<W> {
    fn new(writer: W) -> Self {
        Self {
            writer,
            buf: [0; 3],
            buf_len: 0,
        }
    }

    fn encode_block(&mut self) -> std::io::Result<()> {
        let b = &self.buf;
        let n = self.buf_len;
        let mut out = [b'='; 4];
        if n >= 1 {
            out[0] = BASE64_CHARS[(b[0] >> 2) as usize];
        }
        if n >= 1 {
            out[1] =
                BASE64_CHARS[(((b[0] & 0x03) << 4) | if n >= 2 { b[1] >> 4 } else { 0 }) as usize];
        }
        if n >= 2 {
            out[2] =
                BASE64_CHARS[(((b[1] & 0x0f) << 2) | if n >= 3 { b[2] >> 6 } else { 0 }) as usize];
        }
        if n >= 3 {
            out[3] = BASE64_CHARS[(b[2] & 0x3f) as usize];
        }
        self.writer.write_all(&out)?;
        self.buf_len = 0;
        Ok(())
    }

    fn finish(mut self) -> std::io::Result<()> {
        if self.buf_len > 0 {
            self.encode_block()?;
        }
        Ok(())
    }
}

impl<W: std::io::Write> std::io::Write for Base64Encoder<W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let mut written = 0;
        for &byte in data {
            self.buf[self.buf_len] = byte;
            self.buf_len += 1;
            if self.buf_len == 3 {
                self.encode_block()?;
            }
            written += 1;
        }
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

// ---- Gerrit API response types ----

#[derive(Debug, Deserialize)]
pub struct AccountInfo {
    #[serde(rename = "_account_id")]
    pub account_id: i64,
    pub name: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct RevisionInfo {
    #[serde(rename = "_number")]
    pub number: Option<i32>,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub commit: Option<CommitInfo>,
}

#[derive(Debug, Deserialize)]
pub struct CommitInfo {
    pub subject: Option<String>,
    pub message: Option<String>,
    pub author: Option<GitPersonInfo>,
    pub committer: Option<GitPersonInfo>,
}

#[derive(Debug, Deserialize)]
pub struct GitPersonInfo {
    pub name: Option<String>,
    pub email: Option<String>,
    pub date: Option<String>,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct CommentRange {
    pub start_line: i32,
    pub start_character: i32,
    pub end_line: i32,
    pub end_character: i32,
}

#[derive(Debug, Deserialize)]
pub struct ChangeMessageInfo {
    pub id: Option<String>,
    pub author: Option<AccountInfo>,
    pub date: Option<String>,
    pub message: Option<String>,
    #[serde(rename = "_revision_number")]
    pub revision_number: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_xssi_standard_prefix() {
        let body = ")]}'\n\"3.9.1\"";
        assert_eq!(strip_xssi_prefix(body), "\"3.9.1\"");
    }

    #[test]
    fn strip_xssi_no_quote() {
        let body = ")]}\n{\"foo\": 1}";
        assert_eq!(strip_xssi_prefix(body), "{\"foo\": 1}");
    }

    #[test]
    fn strip_xssi_no_prefix() {
        let body = "{\"foo\": 1}";
        assert_eq!(strip_xssi_prefix(body), "{\"foo\": 1}");
    }

    #[test]
    fn strip_xssi_empty_body() {
        assert_eq!(strip_xssi_prefix(""), "");
    }

    #[test]
    fn deserialize_account_info() {
        let json = r#"{
            "_account_id": 1000096,
            "name": "Alice Smith",
            "email": "alice@example.com",
            "username": "alice"
        }"#;
        let account: AccountInfo = serde_json::from_str(json).unwrap();
        assert_eq!(account.account_id, 1000096);
        assert_eq!(account.name.as_deref(), Some("Alice Smith"));
        assert_eq!(account.email.as_deref(), Some("alice@example.com"));
    }

    #[test]
    fn deserialize_change_info() {
        let json = r#"{
            "id": "proj~main~Iabcdef",
            "project": "proj",
            "branch": "main",
            "change_id": "Iabcdef",
            "subject": "Fix bug",
            "status": "NEW",
            "_number": 12345,
            "owner": {
                "_account_id": 1000096,
                "name": "Alice"
            },
            "current_revision": "abc123",
            "revisions": {
                "abc123": {
                    "_number": 1,
                    "ref": "refs/changes/45/12345/1",
                    "commit": {
                        "subject": "Fix bug",
                        "message": "Fix bug\n\nChange-Id: Iabcdef\n"
                    }
                }
            }
        }"#;
        let change: ChangeInfo = serde_json::from_str(json).unwrap();
        assert_eq!(change.number, Some(12345));
        assert_eq!(change.project.as_deref(), Some("proj"));
        assert!(change.revisions.is_some());
        let revisions = change.revisions.unwrap();
        assert!(revisions.contains_key("abc123"));
    }

    #[test]
    fn deserialize_comment_info() {
        let json = r#"{
            "id": "comment001",
            "path": "src/main.rs",
            "line": 42,
            "message": "Fix this.",
            "updated": "2025-02-10 14:32:00.000000000",
            "author": {
                "_account_id": 1000097,
                "name": "Bob"
            },
            "patch_set": 3,
            "unresolved": true
        }"#;
        let comment: CommentInfo = serde_json::from_str(json).unwrap();
        assert_eq!(comment.id.as_deref(), Some("comment001"));
        assert_eq!(comment.line, Some(42));
        assert_eq!(comment.unresolved, Some(true));
    }

    #[test]
    fn deserialize_change_message() {
        let json = r#"{
            "id": "msg001",
            "author": {
                "_account_id": 1000097,
                "name": "Bob"
            },
            "date": "2025-02-10 14:30:00.000000000",
            "message": "Patch Set 3: Code-Review-1",
            "_revision_number": 3
        }"#;
        let msg: ChangeMessageInfo = serde_json::from_str(json).unwrap();
        assert_eq!(msg.id.as_deref(), Some("msg001"));
        assert_eq!(msg.revision_number, Some(3));
    }

    #[test]
    fn base64_encode_basic() {
        assert_eq!(base64_encode("user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64_encode("a"), "YQ==");
        assert_eq!(base64_encode("ab"), "YWI=");
        assert_eq!(base64_encode("abc"), "YWJj");
    }

    #[test]
    fn auth_headers_basic() {
        let creds = Credentials {
            username: "user".into(),
            password: "pass".into(),
            auth_type: AuthType::Basic,
        };
        let client = GerritClient::new(
            Url::parse("https://example.com").unwrap(),
            Some(creds),
            true,
        )
        .unwrap();
        let headers = client.auth_headers();
        let auth = headers.get(AUTHORIZATION).unwrap().to_str().unwrap();
        assert!(auth.starts_with("Basic "), "expected Basic auth: {auth}");
        assert_eq!(auth, "Basic dXNlcjpwYXNz");
    }

    #[test]
    fn auth_headers_bearer() {
        let creds = Credentials {
            username: "user".into(),
            password: "my-token-123".into(),
            auth_type: AuthType::Bearer,
        };
        let client = GerritClient::new(
            Url::parse("https://example.com").unwrap(),
            Some(creds),
            true,
        )
        .unwrap();
        let headers = client.auth_headers();
        let auth = headers.get(AUTHORIZATION).unwrap().to_str().unwrap();
        assert_eq!(auth, "Bearer my-token-123");
    }

    #[test]
    fn api_url_preserves_base_path() {
        let client = GerritClient::new(
            Url::parse("https://example.com/gerrit/").unwrap(),
            None,
            true,
        )
        .unwrap();
        let url = client.api_url("/changes/").unwrap();
        assert_eq!(url.path(), "/gerrit/changes/");
    }

    #[test]
    fn api_url_with_auth_prefix() {
        let creds = Credentials {
            username: "user".into(),
            password: "pass".into(),
            auth_type: AuthType::Basic,
        };
        let client = GerritClient::new(
            Url::parse("https://example.com/gerrit/").unwrap(),
            Some(creds),
            true,
        )
        .unwrap();
        let url = client.api_url("/changes/").unwrap();
        assert_eq!(url.path(), "/gerrit/a/changes/");
    }

    #[test]
    fn auth_type_default_is_basic() {
        assert_eq!(AuthType::default(), AuthType::Basic);
    }

    #[test]
    fn gerrit_error_retryable_server_5xx() {
        let err = GerritError::ServerError {
            status: 500,
            body: "internal".into(),
        };
        assert!(err.is_retryable());
    }

    #[test]
    fn gerrit_error_retryable_503() {
        let err = GerritError::ServerError {
            status: 503,
            body: "unavailable".into(),
        };
        assert!(err.is_retryable());
    }

    #[test]
    fn gerrit_error_retryable_network() {
        let err = GerritError::Network("connection reset".into());
        assert!(err.is_retryable());
    }

    #[test]
    fn gerrit_error_not_retryable_auth() {
        let err = GerritError::AuthFailed { status: 401 };
        assert!(!err.is_retryable());
    }

    #[test]
    fn gerrit_error_not_retryable_404() {
        let err = GerritError::NotFound;
        assert!(!err.is_retryable());
    }

    #[test]
    fn gerrit_error_not_retryable_4xx() {
        let err = GerritError::ServerError {
            status: 400,
            body: "bad request".into(),
        };
        assert!(!err.is_retryable());
    }
}
