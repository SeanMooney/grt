// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use std::path::Path;

use anyhow::{Context, Result};
use tracing::debug;

use crate::config::{self, CliOverrides, GerritConfig};
use crate::gerrit::{Credentials, GerritClient};
use crate::git::GitRepo;
use crate::subprocess;

/// Indicates where credentials were sourced from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialSource {
    /// Loaded from `~/.config/grt/credentials.toml`.
    File,
    /// Obtained via `git credential fill`.
    GitHelper,
}

/// Application context holding shared resources.
pub struct App {
    pub config: GerritConfig,
    pub git: Option<GitRepo>,
    pub gerrit: GerritClient,
    credential_source: Option<CredentialSource>,
    insecure: bool,
}

impl App {
    /// Open a repo (if available), load config, and create a Gerrit client.
    ///
    /// If `work_dir` is not inside a git repository, `git` will be `None` and
    /// config will be loaded using the credentials-file default and CLI flags.
    /// Commands that require git (push, setup) must call
    /// `app.git.as_ref().ok_or_else(|| anyhow!("not in a git repo"))`.
    pub fn new(work_dir: &Path, cli: &CliOverrides) -> Result<Self> {
        let (git, root) = match GitRepo::open(work_dir) {
            Ok(git) => {
                let root = git.root()?;
                (Some(git), root)
            }
            Err(_) => (None, work_dir.to_path_buf()),
        };

        let config = match &git {
            Some(g) => config::load_config(&root, |key| g.config_value(key), cli)?,
            None => config::load_config(&root, |_| None, cli)?,
        };

        if config.host.is_empty() {
            let config_path = dirs::config_dir()
                .map(|d| d.join("grt").join("credentials.toml").display().to_string())
                .unwrap_or_else(|| "~/.config/grt/credentials.toml".to_string());
            anyhow::bail!(
                "no Gerrit host configured. Options:\n\
                   1. Run grt from inside a repo that has a .gitreview file\n\
                   2. Set gitreview.host in git config\n\
                   3. Use --server <host> to specify the host directly\n\
                   4. Mark a server as 'default = true' in {config_path}\n\
                 \n\
                 Example credentials.toml entry:\n\
                 \n\
                   [[server]]\n\
                   name = \"review.opendev.org\"\n\
                   username = \"you\"\n\
                   password = \"your-http-password\"\n\
                   default = true"
            );
        }

        if config.project.is_empty() && git.is_some() {
            anyhow::bail!(
                "no Gerrit project configured. Options:\n\
                   1. Set project in .gitreview\n\
                   2. Set gitreview.project in git config\n\
                   3. Use --project <project> to specify the project directly"
            );
        }

        let base_url = config.gerrit_base_url()?;
        let gerrit = GerritClient::new(base_url, None, config.ssl_verify)?;

        Ok(Self {
            config,
            git,
            gerrit,
            credential_source: None,
            insecure: cli.insecure,
        })
    }

    /// Return the git repo, or fail with a helpful error for commands that require one.
    pub fn require_git(&self) -> Result<&GitRepo> {
        self.git.as_ref().ok_or_else(|| {
            anyhow::anyhow!("not in a git repo; this command requires a git repository")
        })
    }

    /// Acquire credentials: try credentials.toml first, then git credential helper.
    ///
    /// When credentials come from the git helper, a successful `authenticate_and_verify`
    /// will call `git credential approve` so the helper can cache them.
    ///
    /// Refuses to send credentials over plain HTTP unless `--insecure` was passed.
    pub fn authenticate(&mut self) -> Result<()> {
        // The REST API always uses HTTPS unless scheme is explicitly "http".
        // SSH scheme (the default) maps to HTTPS for API requests, so only
        // block when scheme is literally "http" without --insecure.
        if self.config.scheme == "http" && !self.insecure {
            anyhow::bail!(
                "refusing to send credentials over plain HTTP (scheme: {}). \
                 Use --insecure to override, or switch to HTTPS",
                self.config.scheme,
            );
        }

        // Try credentials.toml first
        if let Some(config_dir) = dirs::config_dir() {
            match config::load_credentials(&self.config.host, &config_dir) {
                Ok(Some(loaded)) => {
                    debug!("credentials loaded from credentials.toml");
                    self.set_credentials(
                        loaded.username,
                        loaded.password,
                        loaded.auth_type,
                        CredentialSource::File,
                    )?;
                    return Ok(());
                }
                Ok(None) => {
                    debug!("no matching entry in credentials.toml, trying git credential helper");
                }
                Err(e) => {
                    return Err(e).context("loading credentials from credentials.toml");
                }
            }
        }

        // Fall back to git credential helper (always Basic auth)
        let url = self.config.gerrit_base_url()?.to_string();
        let root = self.require_git()?.root()?;
        let (username, password) = subprocess::git_credential_fill(&url, &root)
            .context("acquiring credentials")?
            .ok_or_else(|| {
                anyhow::anyhow!("git credential helper did not return username/password")
            })?;
        self.set_credentials(
            username,
            password,
            crate::gerrit::AuthType::Basic,
            CredentialSource::GitHelper,
        )?;
        Ok(())
    }

    /// Authenticate and verify credentials by calling `/accounts/self`.
    ///
    /// On success with git-helper-sourced credentials, calls `git credential approve`.
    /// On failure with git-helper-sourced credentials, calls `git credential reject`.
    pub async fn authenticate_and_verify(&mut self) -> Result<()> {
        self.authenticate()?;

        match self.gerrit.get_self_account().await {
            Ok(account) => {
                let name = account.name.as_deref().unwrap_or("unknown");
                debug!(user = name, "credentials verified");

                if self.credential_source == Some(CredentialSource::GitHelper) {
                    self.approve_git_credentials();
                }
                Ok(())
            }
            Err(e) => {
                if self.credential_source == Some(CredentialSource::GitHelper) {
                    self.reject_git_credentials();
                }
                Err(e).context("verifying credentials against Gerrit")
            }
        }
    }

    fn set_credentials(
        &mut self,
        username: String,
        password: String,
        auth_type: crate::gerrit::AuthType,
        source: CredentialSource,
    ) -> Result<()> {
        self.gerrit.set_credentials(Credentials {
            username,
            password,
            auth_type,
        });
        self.credential_source = Some(source);
        // Re-create client with auth prefix
        let base_url = self.config.gerrit_base_url()?;
        self.gerrit = GerritClient::new(
            base_url,
            self.gerrit.credentials().cloned(),
            self.config.ssl_verify,
        )?;
        Ok(())
    }

    fn approve_git_credentials(&self) {
        if let Some(creds) = self.gerrit.credentials() {
            if let Ok(url) = self.config.gerrit_base_url() {
                if let Some(git) = &self.git {
                    if let Ok(root) = git.root() {
                        let _ = subprocess::git_credential_approve(
                            url.as_str(),
                            &creds.username,
                            &creds.password,
                            &root,
                        );
                    }
                }
            }
        }
    }

    fn reject_git_credentials(&self) {
        if let Some(creds) = self.gerrit.credentials() {
            if let Ok(url) = self.config.gerrit_base_url() {
                if let Some(git) = &self.git {
                    if let Ok(root) = git.root() {
                        let _ = subprocess::git_credential_reject(
                            url.as_str(),
                            &creds.username,
                            &creds.password,
                            &root,
                        );
                    }
                }
            }
        }
    }
}
