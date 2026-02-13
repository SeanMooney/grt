// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Wrapper around a gix repository providing read operations.
pub struct GitRepo {
    repo: gix::Repository,
}

impl GitRepo {
    /// Discover and open a git repository at or above the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let repo = gix::discover(path).context("discovering git repository")?;
        Ok(Self { repo })
    }

    /// Return the worktree root path.
    pub fn root(&self) -> Result<PathBuf> {
        self.repo
            .workdir()
            .map(|p| p.to_path_buf())
            .ok_or_else(|| anyhow::anyhow!("repository is bare (no worktree)"))
    }

    /// Return the current branch name (the short ref, e.g. "main").
    /// Returns an error if HEAD is detached.
    pub fn current_branch(&self) -> Result<String> {
        let head = self.repo.head_ref().context("reading HEAD ref")?;
        match head {
            Some(reference) => {
                let full_name = reference.name().as_bstr().to_string();
                let branch = full_name.strip_prefix("refs/heads/").unwrap_or(&full_name);
                Ok(branch.to_string())
            }
            None => anyhow::bail!("HEAD is detached"),
        }
    }

    /// Read a git config value by its dotted key (e.g. "gitreview.host").
    pub fn config_value(&self, key: &str) -> Option<String> {
        let config = self.repo.config_snapshot();
        config.string(key).map(|v| v.to_string())
    }

    /// Return the path to the hooks directory, respecting `core.hooksPath`.
    pub fn hooks_dir(&self) -> Result<PathBuf> {
        if let Some(custom) = self.config_value("core.hooksPath") {
            let custom_path = Path::new(&custom);
            if custom_path.is_absolute() {
                return Ok(custom_path.to_path_buf());
            }
            // Relative to worktree root
            let root = self.root()?;
            return Ok(root.join(custom_path));
        }

        let git_dir = self.repo.git_dir().to_path_buf();
        Ok(git_dir.join("hooks"))
    }

    /// Get the upstream tracking remote and branch for the current branch.
    ///
    /// Reads `branch.<name>.remote` and `branch.<name>.merge` from git config.
    /// Returns `Some((remote, branch))` if both are configured, `None` otherwise.
    pub fn upstream_branch(&self) -> Result<Option<(String, String)>> {
        let branch = self.current_branch()?;
        let remote_key = format!("branch.{branch}.remote");
        let merge_key = format!("branch.{branch}.merge");

        let remote = match self.config_value(&remote_key) {
            Some(r) => r,
            None => return Ok(None),
        };

        let merge = match self.config_value(&merge_key) {
            Some(m) => {
                // Convert refs/heads/main -> main
                m.strip_prefix("refs/heads/").unwrap_or(&m).to_string()
            }
            None => return Ok(None),
        };

        Ok(Some((remote, merge)))
    }

    /// Check if the worktree has uncommitted changes (staged or unstaged).
    pub fn is_dirty(&self) -> Result<bool> {
        // Use git status subprocess for reliability — gix's status API
        // requires careful feature flag management and is complex for MVP.
        let output = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(self.root()?)
            .output()
            .context("running git status")?;

        Ok(!output.stdout.is_empty())
    }

    /// Return the message of the HEAD commit.
    pub fn head_commit_message(&self) -> Result<String> {
        let head = self.repo.head_commit().context("reading HEAD commit")?;
        let message = head.message_raw().context("reading commit message")?;
        Ok(message.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo(dir: &Path) {
        std::process::Command::new("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(dir)
            .output()
            .expect("git init failed");
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .expect("git config failed");
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .expect("git config failed");
        // Create an initial commit so HEAD exists
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "initial"])
            .current_dir(dir)
            .output()
            .expect("git commit failed");
    }

    #[test]
    fn open_valid_repo() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let repo = GitRepo::open(dir.path());
        assert!(repo.is_ok());
    }

    #[test]
    fn open_nonexistent_path() {
        let result = GitRepo::open(Path::new("/nonexistent/path/that/does/not/exist"));
        assert!(result.is_err());
    }

    #[test]
    fn open_non_repo() {
        let dir = tempfile::tempdir().unwrap();
        let result = GitRepo::open(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn current_branch_main() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let repo = GitRepo::open(dir.path()).unwrap();
        assert_eq!(repo.current_branch().unwrap(), "main");
    }

    #[test]
    fn repo_root_matches() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let repo = GitRepo::open(dir.path()).unwrap();
        let root = repo.root().unwrap();
        // Canonicalize to handle symlinks (e.g. /tmp -> /private/tmp on macOS)
        assert_eq!(
            root.canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn upstream_branch_configured() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        // Set up tracking
        std::process::Command::new("git")
            .args(["config", "branch.main.remote", "origin"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "branch.main.merge", "refs/heads/develop"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let repo = GitRepo::open(dir.path()).unwrap();
        let result = repo.upstream_branch().unwrap();
        assert_eq!(result, Some(("origin".into(), "develop".into())));
    }

    #[test]
    fn upstream_branch_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let repo = GitRepo::open(dir.path()).unwrap();
        let result = repo.upstream_branch().unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn hooks_dir_default() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let repo = GitRepo::open(dir.path()).unwrap();
        let hooks = repo.hooks_dir().unwrap();
        assert!(
            hooks.ends_with("hooks"),
            "hooks dir should end with 'hooks': {hooks:?}"
        );
    }
}
