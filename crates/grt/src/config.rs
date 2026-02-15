// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use url::Url;

use crate::gerrit::AuthType;

/// Configuration for connecting to a Gerrit instance.
#[derive(Debug, Clone)]
pub struct GerritConfig {
    pub host: String,
    /// SSH port from `.gitreview` — used for git remote URLs, not REST API.
    pub ssh_port: Option<u16>,
    /// HTTP(S) port for the REST API. Only set via grt TOML config or CLI override.
    /// When `None`, the standard port for `scheme` is used (443 for https, 80 for http).
    pub http_port: Option<u16>,
    pub project: String,
    pub branch: String,
    pub remote: String,
    pub scheme: String,
    pub default_rebase: bool,
    pub track: bool,
    pub notopic: bool,
    pub usepushurl: bool,
    pub ssl_verify: bool,
    pub username: Option<String>,
}

impl GerritConfig {
    /// Construct the base URL for Gerrit REST API requests.
    ///
    /// Uses `http_port` if explicitly set, otherwise the standard port for the scheme.
    /// The `.gitreview` port (SSH) is intentionally not used here.
    pub fn gerrit_base_url(&self) -> Result<Url> {
        // REST API always uses HTTPS regardless of git transport scheme.
        // Only use HTTP if explicitly configured (scheme = "http").
        let api_scheme = if self.scheme == "http" {
            "http"
        } else {
            "https"
        };
        let url_str = match self.http_port {
            Some(port) => format!("{api_scheme}://{}:{}", self.host, port),
            None => format!("{api_scheme}://{}", self.host),
        };
        Url::parse(&url_str).context("constructing Gerrit base URL")
    }

    /// Build a remote URL from the config fields.
    ///
    /// Format: `scheme://[username@]host[:port]/project`
    ///
    /// Uses `ssh_port` for SSH scheme, `http_port` for HTTP(S) scheme.
    /// Includes `username` in the URL when available (required for SSH).
    pub fn make_remote_url(&self) -> String {
        let mut url = format!("{}://", self.scheme);

        if let Some(ref username) = self.username {
            url.push_str(username);
            url.push('@');
        }

        url.push_str(&self.host);

        match self.scheme.as_str() {
            "ssh" => {
                if let Some(port) = self.ssh_port {
                    url.push_str(&format!(":{port}"));
                }
            }
            _ => {
                if let Some(port) = self.http_port {
                    url.push_str(&format!(":{port}"));
                }
            }
        }

        url.push('/');
        url.push_str(&self.project);

        url
    }
}

impl Default for GerritConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            ssh_port: None,
            http_port: None,
            project: String::new(),
            branch: String::from("master"),
            remote: String::from("gerrit"),
            scheme: String::from("ssh"),
            default_rebase: true,
            track: false,
            notopic: false,
            usepushurl: false,
            ssl_verify: true,
            username: None,
        }
    }
}

/// URL rewrite rules parsed from `git config --list`.
///
/// Mirrors git's `url.<base>.insteadOf` and `url.<base>.pushInsteadOf` config.
#[derive(Debug, Default)]
pub struct UrlRewrites {
    /// `url.<new>.insteadOf = <old>` — applies to both fetch and push.
    pub instead_of: Vec<(String, String)>,
    /// `url.<new>.pushInsteadOf = <old>` — applies only to push URLs.
    pub push_instead_of: Vec<(String, String)>,
}

/// Parse `git config --list` output for URL rewrite rules.
///
/// Looks for entries matching:
/// - `url.<base>.insteadof=<prefix>` (case-insensitive key)
/// - `url.<base>.pushinsteadof=<prefix>` (case-insensitive key)
pub fn populate_rewrites(config_list: &str) -> UrlRewrites {
    let mut rewrites = UrlRewrites::default();

    for line in config_list.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let lower_key = key.to_lowercase();

        if let Some(rest) = lower_key.strip_prefix("url.") {
            if let Some(base) = rest.strip_suffix(".insteadof") {
                // Recover original-case base from the key
                let original_base = &key[4..4 + base.len()];
                rewrites
                    .instead_of
                    .push((value.to_string(), original_base.to_string()));
            } else if let Some(base) = rest.strip_suffix(".pushinsteadof") {
                let original_base = &key[4..4 + base.len()];
                rewrites
                    .push_instead_of
                    .push((value.to_string(), original_base.to_string()));
            }
        }
    }

    rewrites
}

/// Apply URL rewrite rules using longest-match semantics.
///
/// If `for_push` is true, `pushInsteadOf` rules take precedence over `insteadOf`.
/// Otherwise only `insteadOf` rules are applied.
pub fn alias_url(url: &str, rewrites: &UrlRewrites, for_push: bool) -> String {
    // Try pushInsteadOf first when pushing
    if for_push {
        if let Some(result) = longest_match_replace(url, &rewrites.push_instead_of) {
            return result;
        }
    }

    // Fall back to insteadOf
    if let Some(result) = longest_match_replace(url, &rewrites.instead_of) {
        return result;
    }

    url.to_string()
}

/// Find the longest matching prefix and replace it.
fn longest_match_replace(url: &str, rules: &[(String, String)]) -> Option<String> {
    let mut best_match: Option<(&str, &str)> = None;
    let mut best_len = 0;

    for (prefix, replacement) in rules {
        if url.starts_with(prefix.as_str()) && prefix.len() > best_len {
            best_len = prefix.len();
            best_match = Some((prefix.as_str(), replacement.as_str()));
        }
    }

    best_match.map(|(prefix, replacement)| format!("{}{}", replacement, &url[prefix.len()..]))
}

/// Resolve the effective remote URL, applying URL rewrites.
///
/// Tries `remote.get-url --push` first, then `remote.get-url`, applying rewrites.
pub fn get_remote_url(
    remote: &str,
    rewrites: &UrlRewrites,
    git_remote_url: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    if let Some(url) = git_remote_url(remote) {
        let rewritten = alias_url(&url, rewrites, true);
        return Some(rewritten);
    }
    None
}

/// Values that can be overridden via CLI flags.
#[derive(Debug, Default)]
pub struct CliOverrides {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub remote: Option<String>,
    pub scheme: Option<String>,
    /// Use push URL for remote operations (--use-pushurl).
    pub use_pushurl: Option<bool>,
    /// Allow sending credentials over plain HTTP (no TLS).
    pub insecure: bool,
}

/// A single server entry in `credentials.toml`.
#[derive(Deserialize)]
struct ServerCredential {
    name: String,
    username: String,
    password: String,
    /// Authentication type: "basic" (default) or "bearer".
    auth_type: Option<String>,
}

/// Top-level structure of `~/.config/grt/credentials.toml`.
#[derive(Deserialize)]
struct CredentialsFile {
    server: Vec<ServerCredential>,
}

/// Loaded credential set from `credentials.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedCredentials {
    pub username: String,
    pub password: String,
    pub auth_type: AuthType,
}

/// Load credentials for `host` from `<config_dir>/grt/credentials.toml`.
///
/// Returns `Ok(None)` if the file is missing or no entry matches `host`.
/// Returns `Err` if the file has bad permissions (must be `0600` on Unix) or invalid TOML.
pub fn load_credentials(host: &str, config_dir: &Path) -> Result<Option<LoadedCredentials>> {
    let cred_path = config_dir.join("grt").join("credentials.toml");
    if !cred_path.exists() {
        return Ok(None);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(&cred_path)
            .with_context(|| format!("reading metadata for {}", cred_path.display()))?;
        let mode = metadata.mode() & 0o777;
        if mode != 0o600 {
            anyhow::bail!(
                "{} has permissions {:04o}, expected 0600. \
                 Fix with: chmod 600 {}",
                cred_path.display(),
                mode,
                cred_path.display(),
            );
        }
    }

    let content = std::fs::read_to_string(&cred_path)
        .with_context(|| format!("reading {}", cred_path.display()))?;
    let creds: CredentialsFile =
        toml::from_str(&content).with_context(|| format!("parsing {}", cred_path.display()))?;

    for server in &creds.server {
        if server.name == host {
            let auth_type = match server.auth_type.as_deref() {
                Some("bearer") => AuthType::Bearer,
                _ => AuthType::Basic,
            };
            return Ok(Some(LoadedCredentials {
                username: server.username.clone(),
                password: server.password.clone(),
                auth_type,
            }));
        }
    }

    Ok(None)
}

/// Parse a `.gitreview` INI file. Expects a `[gerrit]` section with key=value pairs.
pub fn parse_gitreview(content: &str) -> Result<HashMap<String, String>> {
    let mut in_gerrit_section = false;
    let mut found_section = false;
    let mut values = HashMap::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }

        if trimmed.starts_with('[') {
            in_gerrit_section = trimmed.eq_ignore_ascii_case("[gerrit]");
            if in_gerrit_section {
                found_section = true;
            }
            continue;
        }

        if in_gerrit_section {
            if let Some((key, value)) = trimmed.split_once('=').or_else(|| trimmed.split_once(':'))
            {
                values.insert(key.trim().to_lowercase(), value.trim().to_string());
            }
        }
    }

    if !found_section {
        anyhow::bail!("missing [gerrit] section in .gitreview");
    }

    Ok(values)
}

/// Strip a trailing `.git` suffix from a project name.
///
/// Gerrit project names in `.gitreview` files often include `.git` (because they
/// double as git remote paths), but the Gerrit REST API expects the bare project
/// name without the suffix. This matches git-review's behavior.
fn strip_git_suffix(project: &str) -> String {
    project.strip_suffix(".git").unwrap_or(project).to_string()
}

/// Parse a string value as a boolean.
///
/// Returns `false` for `"0"`, `"false"`, and `"no"` (case-insensitive).
/// Returns `true` for all other values.
fn parse_bool_value(value: &str) -> bool {
    !matches!(value.trim().to_lowercase().as_str(), "0" | "false" | "no")
}

/// Load configuration by layering sources: .gitreview, grt config, git config, CLI overrides.
pub fn load_config(
    repo_root: &Path,
    git_config_value: impl Fn(&str) -> Option<String>,
    cli: &CliOverrides,
) -> Result<GerritConfig> {
    let mut config = GerritConfig::default();

    // Layer 1: .gitreview file
    let gitreview_path = repo_root.join(".gitreview");
    if gitreview_path.exists() {
        let content =
            std::fs::read_to_string(&gitreview_path).context("reading .gitreview file")?;
        let values = parse_gitreview(&content)?;

        if let Some(host) = values.get("host") {
            config.host = host.clone();
        }
        if let Some(port) = values.get("port") {
            config.ssh_port = Some(port.parse::<u16>().context("parsing port in .gitreview")?);
        }
        if let Some(project) = values.get("project") {
            config.project = strip_git_suffix(project);
        }
        if let Some(branch) = values.get("defaultbranch") {
            config.branch = branch.clone();
        }
        if let Some(remote) = values.get("defaultremote") {
            config.remote = remote.clone();
        }
        if let Some(scheme) = values.get("scheme") {
            config.scheme = scheme.clone();
        }
        if let Some(val) = values.get("defaultrebase") {
            config.default_rebase = parse_bool_value(val);
        }
        if let Some(val) = values.get("track") {
            config.track = parse_bool_value(val);
        }
        if let Some(val) = values.get("notopic") {
            config.notopic = parse_bool_value(val);
        }
        if let Some(val) = values.get("usepushurl") {
            config.usepushurl = parse_bool_value(val);
        }
    }

    // Layer 2: grt native TOML config
    if let Some(config_dir) = dirs::config_dir() {
        let toml_path = config_dir.join("grt").join("config.toml");
        if toml_path.exists() {
            let content = std::fs::read_to_string(&toml_path).context("reading grt config.toml")?;
            let table: toml::Table = toml::from_str(&content).context("parsing grt config.toml")?;

            if let Some(gerrit) = table.get("gerrit").and_then(|v| v.as_table()) {
                if let Some(host) = gerrit.get("host").and_then(|v| v.as_str()) {
                    config.host = host.to_string();
                }
                if let Some(port) = gerrit.get("port").and_then(|v| v.as_integer()) {
                    config.http_port = Some(port as u16);
                }
                if let Some(project) = gerrit.get("project").and_then(|v| v.as_str()) {
                    config.project = strip_git_suffix(project);
                }
                if let Some(branch) = gerrit.get("branch").and_then(|v| v.as_str()) {
                    config.branch = branch.to_string();
                }
                if let Some(remote) = gerrit.get("remote").and_then(|v| v.as_str()) {
                    config.remote = remote.to_string();
                }
                if let Some(scheme) = gerrit.get("scheme").and_then(|v| v.as_str()) {
                    config.scheme = scheme.to_string();
                }
            }
        }
    }

    // Layer 3: git config (gitreview.*)
    if let Some(host) =
        git_config_value("gitreview.host").or_else(|| git_config_value("gitreview.hostname"))
    {
        config.host = host;
    }
    if let Some(port) = git_config_value("gitreview.port") {
        config.ssh_port = Some(
            port.parse::<u16>()
                .context("parsing gitreview.port from git config")?,
        );
    }
    if let Some(project) = git_config_value("gitreview.project") {
        config.project = strip_git_suffix(&project);
    }
    if let Some(branch) = git_config_value("gitreview.branch") {
        config.branch = branch;
    }
    if let Some(remote) = git_config_value("gitreview.remote") {
        config.remote = remote;
    }
    if let Some(username) = git_config_value("gitreview.username") {
        config.username = Some(username);
    }

    // SSL verification: git config + environment
    if let Some(ssl) = git_config_value("http.sslVerify") {
        if ssl.eq_ignore_ascii_case("false") {
            config.ssl_verify = false;
        }
    }
    if std::env::var("GIT_SSL_NO_VERIFY").is_ok() {
        config.ssl_verify = false;
    }

    // Layer 4: CLI overrides (highest precedence)
    if let Some(ref host) = cli.host {
        config.host = host.clone();
    }
    if let Some(port) = cli.port {
        config.http_port = Some(port);
    }
    if let Some(ref project) = cli.project {
        config.project = strip_git_suffix(project);
    }
    if let Some(ref branch) = cli.branch {
        config.branch = branch.clone();
    }
    if let Some(ref remote) = cli.remote {
        config.remote = remote.clone();
    }
    if let Some(ref scheme) = cli.scheme {
        config.scheme = scheme.clone();
    }
    if let Some(use_push) = cli.use_pushurl {
        config.usepushurl = use_push;
    }

    // Default SSH port when using ssh scheme
    if config.scheme == "ssh" && config.ssh_port.is_none() {
        config.ssh_port = Some(29418);
    }

    // URL rewriting: insteadOf / pushInsteadOf is handled at the call site
    // via populate_rewrites() + alias_url() since it needs git config --list output.

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gitreview_basic() {
        let content = "\
[gerrit]
host=review.example.com
port=29418
project=my/project
defaultbranch=develop
";
        let values = parse_gitreview(content).unwrap();
        assert_eq!(values.get("host").unwrap(), "review.example.com");
        assert_eq!(values.get("port").unwrap(), "29418");
        assert_eq!(values.get("project").unwrap(), "my/project");
        assert_eq!(values.get("defaultbranch").unwrap(), "develop");
    }

    #[test]
    fn parse_gitreview_minimal() {
        let content = "\
[gerrit]
host=review.example.com
project=my/project
";
        let values = parse_gitreview(content).unwrap();
        assert_eq!(values.get("host").unwrap(), "review.example.com");
        assert_eq!(values.get("project").unwrap(), "my/project");
        assert!(!values.contains_key("port"));
        assert!(!values.contains_key("defaultbranch"));
    }

    #[test]
    fn parse_gitreview_with_spaces() {
        let content = "\
[gerrit]
host = review.example.com
port = 29418
project = my/project
";
        let values = parse_gitreview(content).unwrap();
        assert_eq!(values.get("host").unwrap(), "review.example.com");
        assert_eq!(values.get("port").unwrap(), "29418");
        assert_eq!(values.get("project").unwrap(), "my/project");
    }

    #[test]
    fn parse_gitreview_missing_section() {
        let content = "\
host=review.example.com
project=my/project
";
        let err = parse_gitreview(content).unwrap_err();
        assert!(
            err.to_string().contains("[gerrit]"),
            "error should mention missing section: {err}"
        );
    }

    #[test]
    fn parse_gitreview_empty() {
        let err = parse_gitreview("").unwrap_err();
        assert!(
            err.to_string().contains("[gerrit]"),
            "error should mention missing section: {err}"
        );
    }

    #[test]
    fn config_defaults() {
        let config = GerritConfig::default();
        assert_eq!(config.branch, "master");
        assert_eq!(config.remote, "gerrit");
        assert_eq!(config.scheme, "ssh");
        assert!(config.default_rebase);
        assert!(!config.track);
        assert!(!config.notopic);
        assert!(!config.usepushurl);
        assert!(config.ssl_verify);
        assert!(config.username.is_none());
    }

    #[test]
    fn config_cli_overrides_file() {
        let dir = tempfile::tempdir().unwrap();
        let gitreview = dir.path().join(".gitreview");
        std::fs::write(
            &gitreview,
            "[gerrit]\nhost=file.example.com\nproject=file/project\n",
        )
        .unwrap();

        let cli = CliOverrides {
            host: Some("cli.example.com".into()),
            ..Default::default()
        };

        let config = load_config(dir.path(), |_| None, &cli).unwrap();
        assert_eq!(config.host, "cli.example.com");
        assert_eq!(config.project, "file/project");
    }

    #[test]
    fn gerrit_base_url_with_http_port() {
        let config = GerritConfig {
            host: "review.example.com".into(),
            http_port: Some(8443),
            scheme: "https".into(),
            ..Default::default()
        };
        let url = config.gerrit_base_url().unwrap();
        assert_eq!(url.as_str(), "https://review.example.com:8443/");
    }

    #[test]
    fn gerrit_base_url_no_port() {
        let config = GerritConfig {
            host: "review.example.com".into(),
            scheme: "https".into(),
            ..Default::default()
        };
        let url = config.gerrit_base_url().unwrap();
        assert_eq!(url.as_str(), "https://review.example.com/");
    }

    #[test]
    fn gerrit_base_url_ignores_ssh_port() {
        let config = GerritConfig {
            host: "review.example.com".into(),
            ssh_port: Some(29418),
            scheme: "https".into(),
            ..Default::default()
        };
        let url = config.gerrit_base_url().unwrap();
        assert_eq!(
            url.as_str(),
            "https://review.example.com/",
            "SSH port should not appear in REST API URL"
        );
    }

    #[test]
    fn gerrit_base_url_ssh_scheme_uses_https() {
        let config = GerritConfig {
            host: "review.opendev.org".into(),
            scheme: "ssh".into(),
            ssh_port: Some(29418),
            ..Default::default()
        };
        let url = config.gerrit_base_url().unwrap();
        assert_eq!(
            url.as_str(),
            "https://review.opendev.org/",
            "SSH transport scheme should produce HTTPS REST API URL"
        );
    }

    #[test]
    fn gerrit_base_url_default_scheme_uses_https() {
        let config = GerritConfig {
            host: "review.opendev.org".into(),
            ..Default::default()
        };
        assert_eq!(config.scheme, "ssh", "default scheme should be ssh");
        let url = config.gerrit_base_url().unwrap();
        assert_eq!(
            url.as_str(),
            "https://review.opendev.org/",
            "Default config should produce HTTPS REST API URL"
        );
    }

    #[test]
    fn gerrit_base_url_http_scheme_preserved() {
        let config = GerritConfig {
            host: "localhost".into(),
            scheme: "http".into(),
            http_port: Some(8080),
            ..Default::default()
        };
        let url = config.gerrit_base_url().unwrap();
        assert_eq!(
            url.as_str(),
            "http://localhost:8080/",
            "Explicit HTTP scheme should be preserved for dev/insecure setups"
        );
    }

    #[test]
    fn gitreview_port_goes_to_ssh_port() {
        let dir = tempfile::tempdir().unwrap();
        let gitreview = dir.path().join(".gitreview");
        std::fs::write(
            &gitreview,
            "[gerrit]\nhost=review.opendev.org\nport=29418\nproject=openstack/nova.git\n",
        )
        .unwrap();

        let config = load_config(dir.path(), |_| None, &CliOverrides::default()).unwrap();
        assert_eq!(config.ssh_port, Some(29418));
        assert_eq!(config.http_port, None);
    }

    fn write_credentials_file(dir: &Path, content: &str) -> std::path::PathBuf {
        let grt_dir = dir.join("grt");
        std::fs::create_dir_all(&grt_dir).unwrap();
        let cred_path = grt_dir.join("credentials.toml");
        std::fs::write(&cred_path, content).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&cred_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        cred_path
    }

    #[test]
    fn load_credentials_matching_host() {
        let dir = tempfile::tempdir().unwrap();
        write_credentials_file(
            dir.path(),
            r#"
[[server]]
name = "review.opendev.org"
username = "alice"
password = "secret-token"
"#,
        );

        let result = load_credentials("review.opendev.org", dir.path()).unwrap();
        let loaded = result.expect("should return matching credentials");
        assert_eq!(loaded.username, "alice");
        assert_eq!(loaded.password, "secret-token");
        assert_eq!(loaded.auth_type, AuthType::Basic);
    }

    #[test]
    fn load_credentials_no_match() {
        let dir = tempfile::tempdir().unwrap();
        write_credentials_file(
            dir.path(),
            r#"
[[server]]
name = "review.opendev.org"
username = "alice"
password = "secret-token"
"#,
        );

        let result = load_credentials("other.example.com", dir.path()).unwrap();
        assert_eq!(result, None, "should return None for non-matching host");
    }

    #[test]
    fn load_credentials_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_credentials("review.opendev.org", dir.path()).unwrap();
        assert_eq!(result, None, "should return None when file is missing");
    }

    #[cfg(unix)]
    #[test]
    fn load_credentials_bad_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let cred_path = write_credentials_file(
            dir.path(),
            r#"
[[server]]
name = "review.opendev.org"
username = "alice"
password = "secret-token"
"#,
        );

        // Set bad permissions
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cred_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let err = load_credentials("review.opendev.org", dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("0644"),
            "error should mention actual permissions: {err}"
        );
        assert!(
            err.to_string().contains("0600"),
            "error should mention expected permissions: {err}"
        );
    }

    #[test]
    fn load_credentials_multiple_servers() {
        let dir = tempfile::tempdir().unwrap();
        write_credentials_file(
            dir.path(),
            r#"
[[server]]
name = "review.opendev.org"
username = "alice"
password = "token-1"

[[server]]
name = "review.other.org"
username = "bob"
password = "token-2"
"#,
        );

        let loaded = load_credentials("review.other.org", dir.path())
            .unwrap()
            .expect("should match second server entry");
        assert_eq!(loaded.username, "bob");
        assert_eq!(loaded.password, "token-2");
        assert_eq!(loaded.auth_type, AuthType::Basic);
    }

    #[test]
    fn load_credentials_bearer_auth_type() {
        let dir = tempfile::tempdir().unwrap();
        write_credentials_file(
            dir.path(),
            r#"
[[server]]
name = "review.example.com"
username = "bot"
password = "bearer-token-abc"
auth_type = "bearer"
"#,
        );

        let loaded = load_credentials("review.example.com", dir.path())
            .unwrap()
            .expect("should return matching credentials");
        assert_eq!(loaded.username, "bot");
        assert_eq!(loaded.password, "bearer-token-abc");
        assert_eq!(loaded.auth_type, AuthType::Bearer);
    }

    #[test]
    fn load_credentials_explicit_basic_auth_type() {
        let dir = tempfile::tempdir().unwrap();
        write_credentials_file(
            dir.path(),
            r#"
[[server]]
name = "review.example.com"
username = "alice"
password = "pass"
auth_type = "basic"
"#,
        );

        let loaded = load_credentials("review.example.com", dir.path())
            .unwrap()
            .expect("should return matching credentials");
        assert_eq!(loaded.auth_type, AuthType::Basic);
    }

    #[test]
    fn strip_git_suffix_removes_dotgit() {
        assert_eq!(
            strip_git_suffix("openstack/watcher.git"),
            "openstack/watcher"
        );
    }

    #[test]
    fn strip_git_suffix_no_suffix() {
        assert_eq!(strip_git_suffix("openstack/watcher"), "openstack/watcher");
    }

    #[test]
    fn strip_git_suffix_only_git() {
        assert_eq!(strip_git_suffix(".git"), "");
    }

    #[test]
    fn project_git_suffix_stripped_in_config() {
        let dir = tempfile::tempdir().unwrap();
        let gitreview = dir.path().join(".gitreview");
        std::fs::write(
            &gitreview,
            "[gerrit]\nhost=review.example.com\nproject=openstack/nova.git\n",
        )
        .unwrap();

        let config = load_config(dir.path(), |_| None, &CliOverrides::default()).unwrap();
        assert_eq!(
            config.project, "openstack/nova",
            ".git suffix should be stripped from project name"
        );
    }

    #[test]
    fn parse_gitreview_colon_delimiter() {
        let content = "\
[gerrit]
host: review.example.com
port: 29418
project: my/project
";
        let values = parse_gitreview(content).unwrap();
        assert_eq!(values.get("host").unwrap(), "review.example.com");
        assert_eq!(values.get("port").unwrap(), "29418");
        assert_eq!(values.get("project").unwrap(), "my/project");
    }

    #[test]
    fn parse_gitreview_case_insensitive_keys() {
        let content = "\
[gerrit]
Host=review.example.com
PORT=29418
Project=my/project
DefaultBranch=develop
";
        let values = parse_gitreview(content).unwrap();
        assert_eq!(values.get("host").unwrap(), "review.example.com");
        assert_eq!(values.get("port").unwrap(), "29418");
        assert_eq!(values.get("project").unwrap(), "my/project");
        assert_eq!(values.get("defaultbranch").unwrap(), "develop");
    }

    #[test]
    fn gitreview_hostname_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let gitreview = dir.path().join(".gitreview");
        std::fs::write(&gitreview, "[gerrit]\nproject=my/project\n").unwrap();

        let config = load_config(
            dir.path(),
            |key| match key {
                "gitreview.hostname" => Some("fallback.example.com".to_string()),
                _ => None,
            },
            &CliOverrides::default(),
        )
        .unwrap();
        assert_eq!(config.host, "fallback.example.com");
    }

    #[test]
    fn gitreview_host_takes_precedence_over_hostname() {
        let dir = tempfile::tempdir().unwrap();
        let gitreview = dir.path().join(".gitreview");
        std::fs::write(&gitreview, "[gerrit]\nproject=my/project\n").unwrap();

        let config = load_config(
            dir.path(),
            |key| match key {
                "gitreview.host" => Some("primary.example.com".to_string()),
                "gitreview.hostname" => Some("fallback.example.com".to_string()),
                _ => None,
            },
            &CliOverrides::default(),
        )
        .unwrap();
        assert_eq!(config.host, "primary.example.com");
    }

    #[test]
    fn default_ssh_port_29418() {
        let dir = tempfile::tempdir().unwrap();
        let gitreview = dir.path().join(".gitreview");
        std::fs::write(
            &gitreview,
            "[gerrit]\nhost=review.example.com\nproject=my/project\n",
        )
        .unwrap();

        let config = load_config(dir.path(), |_| None, &CliOverrides::default()).unwrap();
        assert_eq!(config.scheme, "ssh");
        assert_eq!(
            config.ssh_port,
            Some(29418),
            "SSH port should default to 29418 when scheme is ssh"
        );
    }

    #[test]
    fn no_default_ssh_port_when_https() {
        let dir = tempfile::tempdir().unwrap();
        let gitreview = dir.path().join(".gitreview");
        std::fs::write(
            &gitreview,
            "[gerrit]\nhost=review.example.com\nproject=my/project\nscheme=https\n",
        )
        .unwrap();

        let config = load_config(dir.path(), |_| None, &CliOverrides::default()).unwrap();
        assert_eq!(config.scheme, "https");
        assert_eq!(
            config.ssh_port, None,
            "SSH port should not be set when scheme is https and no port specified"
        );
    }

    #[test]
    fn ssl_verify_default_true() {
        let dir = tempfile::tempdir().unwrap();
        let gitreview = dir.path().join(".gitreview");
        std::fs::write(
            &gitreview,
            "[gerrit]\nhost=review.example.com\nproject=my/project\n",
        )
        .unwrap();

        let config = load_config(dir.path(), |_| None, &CliOverrides::default()).unwrap();
        assert!(config.ssl_verify, "ssl_verify should default to true");
    }

    #[test]
    fn ssl_verify_git_config_override() {
        let dir = tempfile::tempdir().unwrap();
        let gitreview = dir.path().join(".gitreview");
        std::fs::write(
            &gitreview,
            "[gerrit]\nhost=review.example.com\nproject=my/project\n",
        )
        .unwrap();

        let config = load_config(
            dir.path(),
            |key| match key {
                "http.sslVerify" => Some("false".to_string()),
                _ => None,
            },
            &CliOverrides::default(),
        )
        .unwrap();
        assert!(
            !config.ssl_verify,
            "ssl_verify should be false when http.sslVerify is false"
        );
    }

    #[test]
    fn parse_bool_value_truthy() {
        assert!(parse_bool_value("1"));
        assert!(parse_bool_value("true"));
        assert!(parse_bool_value("True"));
        assert!(parse_bool_value("TRUE"));
        assert!(parse_bool_value("yes"));
        assert!(parse_bool_value("Yes"));
        assert!(parse_bool_value("anything"));
    }

    #[test]
    fn parse_bool_value_falsy() {
        assert!(!parse_bool_value("0"));
        assert!(!parse_bool_value("false"));
        assert!(!parse_bool_value("False"));
        assert!(!parse_bool_value("FALSE"));
        assert!(!parse_bool_value("no"));
        assert!(!parse_bool_value("No"));
        assert!(!parse_bool_value("NO"));
        assert!(!parse_bool_value("  false  "));
    }

    #[test]
    fn gitreview_bool_fields_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let gitreview = dir.path().join(".gitreview");
        std::fs::write(
            &gitreview,
            "[gerrit]\nhost=review.example.com\nproject=my/project\ndefaultrebase=0\ntrack=true\nnotopic=yes\nusepushurl=false\n",
        )
        .unwrap();

        let config = load_config(dir.path(), |_| None, &CliOverrides::default()).unwrap();
        assert!(!config.default_rebase, "defaultrebase=0 should be false");
        assert!(config.track, "track=true should be true");
        assert!(config.notopic, "notopic=yes should be true");
        assert!(!config.usepushurl, "usepushurl=false should be false");
    }

    #[test]
    fn gitreview_username_from_git_config() {
        let dir = tempfile::tempdir().unwrap();
        let gitreview = dir.path().join(".gitreview");
        std::fs::write(
            &gitreview,
            "[gerrit]\nhost=review.example.com\nproject=my/project\n",
        )
        .unwrap();

        let config = load_config(
            dir.path(),
            |key| match key {
                "gitreview.username" => Some("testuser".to_string()),
                _ => None,
            },
            &CliOverrides::default(),
        )
        .unwrap();
        assert_eq!(config.username.as_deref(), Some("testuser"));
    }

    // === URL rewriting tests ===

    #[test]
    fn populate_rewrites_insteadof() {
        let config_list = "url.ssh://git@github.com/.insteadof=https://github.com/\n";
        let rewrites = populate_rewrites(config_list);
        assert_eq!(rewrites.instead_of.len(), 1);
        assert_eq!(rewrites.instead_of[0].0, "https://github.com/");
        assert_eq!(rewrites.instead_of[0].1, "ssh://git@github.com/");
    }

    #[test]
    fn populate_rewrites_pushinsteadof() {
        let config_list = "url.ssh://git@github.com/.pushinsteadof=https://github.com/\n";
        let rewrites = populate_rewrites(config_list);
        assert_eq!(rewrites.push_instead_of.len(), 1);
        assert_eq!(rewrites.push_instead_of[0].0, "https://github.com/");
        assert_eq!(rewrites.push_instead_of[0].1, "ssh://git@github.com/");
    }

    #[test]
    fn populate_rewrites_empty() {
        let rewrites = populate_rewrites("user.name=Test\nuser.email=test@test.com\n");
        assert!(rewrites.instead_of.is_empty());
        assert!(rewrites.push_instead_of.is_empty());
    }

    #[test]
    fn alias_url_insteadof_applied() {
        let rewrites = UrlRewrites {
            instead_of: vec![(
                "https://github.com/".to_string(),
                "ssh://git@github.com/".to_string(),
            )],
            push_instead_of: vec![],
        };
        assert_eq!(
            alias_url("https://github.com/user/repo", &rewrites, false),
            "ssh://git@github.com/user/repo"
        );
    }

    #[test]
    fn alias_url_longest_match() {
        let rewrites = UrlRewrites {
            instead_of: vec![
                ("https://".to_string(), "http://".to_string()),
                (
                    "https://github.com/".to_string(),
                    "ssh://git@github.com/".to_string(),
                ),
            ],
            push_instead_of: vec![],
        };
        // Longest match should win
        assert_eq!(
            alias_url("https://github.com/user/repo", &rewrites, false),
            "ssh://git@github.com/user/repo"
        );
    }

    #[test]
    fn alias_url_push_takes_precedence() {
        let rewrites = UrlRewrites {
            instead_of: vec![(
                "https://github.com/".to_string(),
                "git://github.com/".to_string(),
            )],
            push_instead_of: vec![(
                "https://github.com/".to_string(),
                "ssh://git@github.com/".to_string(),
            )],
        };
        // For push, pushInsteadOf should take precedence
        assert_eq!(
            alias_url("https://github.com/user/repo", &rewrites, true),
            "ssh://git@github.com/user/repo"
        );
        // For fetch, only insteadOf should apply
        assert_eq!(
            alias_url("https://github.com/user/repo", &rewrites, false),
            "git://github.com/user/repo"
        );
    }

    #[test]
    fn alias_url_no_match() {
        let rewrites = UrlRewrites {
            instead_of: vec![(
                "https://github.com/".to_string(),
                "ssh://git@github.com/".to_string(),
            )],
            push_instead_of: vec![],
        };
        assert_eq!(
            alias_url("https://gitlab.com/user/repo", &rewrites, false),
            "https://gitlab.com/user/repo"
        );
    }

    #[test]
    fn get_remote_url_with_rewrites() {
        let rewrites = UrlRewrites {
            instead_of: vec![],
            push_instead_of: vec![(
                "https://review.example.com/".to_string(),
                "ssh://review.example.com:29418/".to_string(),
            )],
        };
        let url = get_remote_url("gerrit", &rewrites, |_remote| {
            Some("https://review.example.com/project".to_string())
        });
        assert_eq!(
            url.as_deref(),
            Some("ssh://review.example.com:29418/project")
        );
    }

    #[test]
    fn get_remote_url_no_remote() {
        let rewrites = UrlRewrites::default();
        let url = get_remote_url("gerrit", &rewrites, |_| None);
        assert_eq!(url, None);
    }

    // === make_remote_url tests ===

    #[test]
    fn make_remote_url_ssh_with_username() {
        let config = GerritConfig {
            host: "review.example.com".into(),
            scheme: "ssh".into(),
            ssh_port: Some(29418),
            project: "openstack/nova".into(),
            username: Some("alice".into()),
            ..Default::default()
        };
        assert_eq!(
            config.make_remote_url(),
            "ssh://alice@review.example.com:29418/openstack/nova"
        );
    }

    #[test]
    fn make_remote_url_https_no_username() {
        let config = GerritConfig {
            host: "review.example.com".into(),
            scheme: "https".into(),
            project: "openstack/nova".into(),
            ..Default::default()
        };
        assert_eq!(
            config.make_remote_url(),
            "https://review.example.com/openstack/nova"
        );
    }

    #[test]
    fn make_remote_url_https_with_port() {
        let config = GerritConfig {
            host: "review.example.com".into(),
            scheme: "https".into(),
            http_port: Some(8443),
            project: "my/project".into(),
            username: Some("bob".into()),
            ..Default::default()
        };
        assert_eq!(
            config.make_remote_url(),
            "https://bob@review.example.com:8443/my/project"
        );
    }

    #[test]
    fn make_remote_url_ssh_no_port() {
        let config = GerritConfig {
            host: "review.example.com".into(),
            scheme: "ssh".into(),
            ssh_port: None,
            project: "my/project".into(),
            ..Default::default()
        };
        assert_eq!(
            config.make_remote_url(),
            "ssh://review.example.com/my/project"
        );
    }
}
