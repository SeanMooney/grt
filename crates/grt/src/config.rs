use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use url::Url;

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
}

impl GerritConfig {
    /// Construct the base URL for Gerrit REST API requests.
    ///
    /// Uses `http_port` if explicitly set, otherwise the standard port for the scheme.
    /// The `.gitreview` port (SSH) is intentionally not used here.
    pub fn gerrit_base_url(&self) -> Result<Url> {
        let url_str = match self.http_port {
            Some(port) => format!("{}://{}:{}", self.scheme, self.host, port),
            None => format!("{}://{}", self.scheme, self.host),
        };
        Url::parse(&url_str).context("constructing Gerrit base URL")
    }
}

impl Default for GerritConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            ssh_port: None,
            http_port: None,
            project: String::new(),
            branch: String::from("main"),
            remote: String::from("gerrit"),
            scheme: String::from("https"),
        }
    }
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
            if let Some((key, value)) = trimmed.split_once('=') {
                values.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }

    if !found_section {
        anyhow::bail!("missing [gerrit] section in .gitreview");
    }

    Ok(values)
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
            config.ssh_port =
                Some(port.parse::<u16>().context("parsing port in .gitreview")?);
        }
        if let Some(project) = values.get("project") {
            config.project = project.clone();
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
                    config.project = project.to_string();
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
    if let Some(host) = git_config_value("gitreview.host") {
        config.host = host;
    }
    if let Some(port) = git_config_value("gitreview.port") {
        config.ssh_port = Some(
            port.parse::<u16>()
                .context("parsing gitreview.port from git config")?,
        );
    }
    if let Some(project) = git_config_value("gitreview.project") {
        config.project = project;
    }
    if let Some(branch) = git_config_value("gitreview.branch") {
        config.branch = branch;
    }
    if let Some(remote) = git_config_value("gitreview.remote") {
        config.remote = remote;
    }

    // Layer 4: CLI overrides (highest precedence)
    if let Some(ref host) = cli.host {
        config.host = host.clone();
    }
    if let Some(port) = cli.port {
        config.http_port = Some(port);
    }
    if let Some(ref project) = cli.project {
        config.project = project.clone();
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
        assert_eq!(config.branch, "main");
        assert_eq!(config.remote, "gerrit");
        assert_eq!(config.scheme, "https");
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
        // REST API URL should use standard HTTPS port
        let url = config.gerrit_base_url().unwrap();
        assert_eq!(url.as_str(), "https://review.opendev.org/");
    }
}
