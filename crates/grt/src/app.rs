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
    pub git: GitRepo,
    pub gerrit: GerritClient,
    credential_source: Option<CredentialSource>,
    insecure: bool,
}

impl App {
    /// Open a repo, load config, and create a Gerrit client.
    pub fn new(work_dir: &Path, cli: &CliOverrides) -> Result<Self> {
        let git = GitRepo::open(work_dir).context("opening git repository")?;
        let root = git.root()?;

        let config = config::load_config(&root, |key| git.config_value(key), cli)?;

        if config.host.is_empty() {
            anyhow::bail!(
                "no Gerrit host configured. Create a .gitreview file or set gitreview.host in git config"
            );
        }

        let base_url = config.gerrit_base_url()?;
        let gerrit = GerritClient::new(base_url, None)?;

        Ok(Self {
            config,
            git,
            gerrit,
            credential_source: None,
            insecure: cli.insecure,
        })
    }

    /// Acquire credentials: try credentials.toml first, then git credential helper.
    ///
    /// When credentials come from the git helper, a successful `authenticate_and_verify`
    /// will call `git credential approve` so the helper can cache them.
    ///
    /// Refuses to send credentials over plain HTTP unless `--insecure` was passed.
    pub fn authenticate(&mut self) -> Result<()> {
        if self.config.scheme != "https" && !self.insecure {
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
        let root = self.git.root()?;
        let (username, password) =
            subprocess::git_credential_fill(&url, &root).context("acquiring credentials")?;
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
        self.gerrit = GerritClient::new(base_url, self.gerrit.credentials().cloned())?;
        Ok(())
    }

    fn approve_git_credentials(&self) {
        if let Some(creds) = self.gerrit.credentials() {
            if let Ok(url) = self.config.gerrit_base_url() {
                if let Ok(root) = self.git.root() {
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

    fn reject_git_credentials(&self) {
        if let Some(creds) = self.gerrit.credentials() {
            if let Ok(url) = self.config.gerrit_base_url() {
                if let Ok(root) = self.git.root() {
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
