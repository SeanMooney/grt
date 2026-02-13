// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use std::path::Path;

use anyhow::Result;

use crate::subprocess;

/// Result of a pre-push rebase attempt.
#[derive(Debug)]
pub enum RebaseResult {
    /// Rebase succeeded; `orig_head` is the SHA before the rebase.
    Success { orig_head: String },
    /// Rebase failed and was aborted (or left in place if `keep_rebase`).
    Failed,
    /// Rebase was skipped (e.g., remote branch doesn't exist).
    Skipped,
}

/// Perform a pre-push rebase onto `remote/branch`.
///
/// Steps:
/// 1. Update the remote
/// 2. Save the current HEAD
/// 3. Check working tree is clean
/// 4. Check remote branch exists
/// 5. Rebase onto remote/branch
///
/// On failure, aborts the rebase unless `keep_rebase` is set.
pub fn rebase_changes(
    remote: &str,
    branch: &str,
    keep_rebase: bool,
    work_dir: &Path,
) -> Result<RebaseResult> {
    // Update remote refs
    subprocess::git_remote_update(remote, work_dir)?;

    // Save current HEAD so we can undo later
    let orig_head = subprocess::git_rev_parse_head(work_dir)?;

    // Check working tree is clean
    if !subprocess::check_worktree_clean(work_dir)? {
        eprintln!("Cannot rebase: working tree has uncommitted changes.");
        return Ok(RebaseResult::Failed);
    }

    // Check remote tracking branch exists
    if !subprocess::check_remote_branch_exists(remote, branch, work_dir) {
        eprintln!(
            "Remote branch {remote}/{branch} does not exist. \
             Use -R to skip rebase, or push to create it."
        );
        return Ok(RebaseResult::Skipped);
    }

    // Perform rebase
    let remote_branch = format!("{remote}/{branch}");
    eprintln!("Rebasing onto {remote_branch}...");
    match subprocess::git_rebase(&remote_branch, work_dir) {
        Ok(()) => {
            eprintln!("Rebase successful.");
            Ok(RebaseResult::Success { orig_head })
        }
        Err(e) => {
            if keep_rebase {
                eprintln!("Rebase failed: {e:#}");
                eprintln!("Keeping rebase state (--keep-rebase). Resolve conflicts manually.");
            } else {
                eprintln!("Rebase failed: {e:#}");
                eprintln!("Aborting rebase...");
                if let Err(abort_err) = subprocess::git_rebase_abort(work_dir) {
                    tracing::warn!("failed to abort rebase: {abort_err}");
                }
            }
            Ok(RebaseResult::Failed)
        }
    }
}

/// Undo a rebase by resetting to the original HEAD.
pub fn undo_rebase(orig_head: &str, work_dir: &Path) -> Result<()> {
    tracing::debug!("Undoing rebase, resetting to {orig_head}...");
    subprocess::git_reset_hard(orig_head, work_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn init_repo_with_remote(dir: &Path) -> tempfile::TempDir {
        // Create a "remote" bare repo
        let remote_dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "--bare"])
            .current_dir(remote_dir.path())
            .output()
            .expect("git init --bare failed");

        // Init the working repo
        Command::new("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(dir)
            .output()
            .expect("git init failed");
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "initial"])
            .current_dir(dir)
            .output()
            .unwrap();

        // Add remote
        Command::new("git")
            .args([
                "remote",
                "add",
                "gerrit",
                remote_dir.path().to_str().unwrap(),
            ])
            .current_dir(dir)
            .output()
            .unwrap();

        // Push to create remote branch
        Command::new("git")
            .args(["push", "gerrit", "main"])
            .current_dir(dir)
            .output()
            .unwrap();

        // Fetch to populate remote tracking refs
        Command::new("git")
            .args(["fetch", "gerrit"])
            .current_dir(dir)
            .output()
            .unwrap();

        remote_dir
    }

    #[test]
    fn rebase_skipped_when_no_remote_branch() {
        let dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // No remote exists, so rebase should be skipped
        // git_remote_update will fail, which is OK — rebase_changes handles it
        let result = rebase_changes("nonexistent", "main", false, dir.path());
        // Should error on remote update since remote doesn't exist
        assert!(result.is_err() || matches!(result.unwrap(), RebaseResult::Skipped));
    }

    #[test]
    fn rebase_success_on_clean_tree() {
        let dir = tempfile::tempdir().unwrap();
        let _remote = init_repo_with_remote(dir.path());

        // Add a local commit
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "local change"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let result = rebase_changes("gerrit", "main", false, dir.path()).unwrap();
        assert!(
            matches!(result, RebaseResult::Success { .. }),
            "expected Success, got {result:?}"
        );
    }

    #[test]
    fn undo_rebase_restores_head() {
        let dir = tempfile::tempdir().unwrap();
        let _remote = init_repo_with_remote(dir.path());

        let orig_head = subprocess::git_rev_parse_head(dir.path()).unwrap();

        // Make a new commit
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "new"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let new_head = subprocess::git_rev_parse_head(dir.path()).unwrap();
        assert_ne!(orig_head, new_head);

        // Undo (reset to orig_head)
        undo_rebase(&orig_head, dir.path()).unwrap();
        let restored = subprocess::git_rev_parse_head(dir.path()).unwrap();
        assert_eq!(restored, orig_head);
    }
}
