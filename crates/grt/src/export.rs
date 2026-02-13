// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct ExportArgs {
    #[command(subcommand)]
    pub target: ExportTarget,
}

#[derive(Subcommand, Debug)]
pub enum ExportTarget {
    /// Create a git-review symlink to grt
    GitReview {
        /// Remove the symlink instead of creating it
        #[arg(long)]
        clean: bool,
    },
}

/// Resolve the target path for the git-review symlink (`~/.local/bin/git-review`).
pub fn git_review_symlink_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    Ok(home.join(".local").join("bin").join("git-review"))
}

/// Check if a directory is in PATH.
fn is_in_path(dir: &std::path::Path) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for entry in std::env::split_paths(&path_var) {
            if entry == dir {
                return true;
            }
        }
    }
    false
}

pub fn cmd_export(args: &ExportArgs) -> Result<()> {
    match &args.target {
        ExportTarget::GitReview { clean } => {
            let symlink_path = git_review_symlink_path()?;

            if *clean {
                if symlink_path.symlink_metadata().is_ok() {
                    std::fs::remove_file(&symlink_path)
                        .with_context(|| format!("removing {}", symlink_path.display()))?;
                    eprintln!("Removed {}", symlink_path.display());
                } else {
                    eprintln!("{} does not exist", symlink_path.display());
                }
                return Ok(());
            }

            let current_exe =
                std::env::current_exe().context("determining current executable path")?;

            // Ensure parent directory exists
            if let Some(parent) = symlink_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating directory {}", parent.display()))?;
            }

            // Remove existing symlink if present
            if symlink_path.symlink_metadata().is_ok() {
                std::fs::remove_file(&symlink_path)
                    .with_context(|| format!("removing existing {}", symlink_path.display()))?;
            }

            #[cfg(unix)]
            std::os::unix::fs::symlink(&current_exe, &symlink_path)
                .with_context(|| format!("creating symlink {}", symlink_path.display()))?;

            #[cfg(not(unix))]
            anyhow::bail!("symlink creation is only supported on Unix systems");

            eprintln!(
                "Created {} -> {}",
                symlink_path.display(),
                current_exe.display()
            );

            // Warn if ~/.local/bin is not in PATH
            if let Some(parent) = symlink_path.parent() {
                if !is_in_path(parent) {
                    eprintln!(
                        "Warning: {} is not in your PATH. Add it to use `git review`.",
                        parent.display()
                    );
                }
            }

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_review_symlink_path_is_in_local_bin() {
        let path = git_review_symlink_path().unwrap();
        assert!(
            path.ends_with(".local/bin/git-review"),
            "path should end with .local/bin/git-review, got: {path:?}"
        );
    }

    #[test]
    fn export_args_parse_git_review() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: ExportTarget,
        }

        let cli = TestCli::parse_from(["test", "git-review"]);
        assert!(matches!(cli.cmd, ExportTarget::GitReview { clean: false }));
    }

    #[test]
    fn export_args_parse_git_review_clean() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: ExportTarget,
        }

        let cli = TestCli::parse_from(["test", "git-review", "--clean"]);
        assert!(matches!(cli.cmd, ExportTarget::GitReview { clean: true }));
    }
}
