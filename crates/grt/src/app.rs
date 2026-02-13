use std::path::Path;

use anyhow::{Context, Result};

use crate::config::{self, CliOverrides, GerritConfig};
use crate::gerrit::{Credentials, GerritClient};
use crate::git::GitRepo;
use crate::subprocess;

/// Application context holding shared resources.
pub struct App {
    pub config: GerritConfig,
    pub git: GitRepo,
    pub gerrit: GerritClient,
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
        })
    }

    /// Attempt to acquire credentials from git credential helper and configure the client.
    pub fn authenticate(&mut self) -> Result<()> {
        let url = self.config.gerrit_base_url()?.to_string();
        let root = self.git.root()?;
        let (username, password) =
            subprocess::git_credential_fill(&url, &root).context("acquiring credentials")?;
        self.gerrit
            .set_credentials(Credentials { username, password });
        // Re-create client with auth prefix
        let base_url = self.config.gerrit_base_url()?;
        self.gerrit = GerritClient::new(base_url, self.gerrit.credentials().cloned())?;
        Ok(())
    }
}
