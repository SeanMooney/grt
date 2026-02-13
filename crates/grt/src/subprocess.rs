// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

/// Create a git command with locale forced to C for reliable parsing.
fn git_command(args: &[&str], work_dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(work_dir)
        .env("LANG", "C")
        .env("LANGUAGE", "C");
    cmd
}

/// Run a git command and return its stdout output.
pub fn git_output(args: &[&str], work_dir: &Path) -> Result<String> {
    let output = git_command(args, work_dir)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "git {} failed (exit {}): {}",
            args.join(" "),
            output.status.code().unwrap_or(-1),
            stderr.trim()
        );
    }

    let stdout = String::from_utf8(output.stdout).context("git output is not valid UTF-8")?;
    Ok(stdout.trim_end().to_string())
}

/// Run a git command, inheriting stdout/stderr for interactive output.
pub fn git_exec(args: &[&str], work_dir: &Path) -> Result<()> {
    let status = git_command(args, work_dir)
        .status()
        .with_context(|| format!("running git {}", args.join(" ")))?;

    if !status.success() {
        anyhow::bail!(
            "git {} failed (exit {})",
            args.join(" "),
            status.code().unwrap_or(-1)
        );
    }

    Ok(())
}

/// Count unpushed commits between HEAD and a remote tracking branch.
pub fn count_unpushed_commits(remote: &str, branch: &str, work_dir: &Path) -> Result<usize> {
    let remote_ref = format!("remotes/{}/{}", remote, branch);
    let output = git_output(
        &["log", "HEAD", "--not", &remote_ref, "--oneline"],
        work_dir,
    );

    match output {
        Ok(text) => {
            if text.is_empty() {
                Ok(0)
            } else {
                Ok(text.lines().count())
            }
        }
        Err(_) => {
            // Remote branch may not exist yet; count all commits
            let text = git_output(&["log", "--oneline"], work_dir)?;
            if text.is_empty() {
                Ok(0)
            } else {
                Ok(text.lines().count())
            }
        }
    }
}

/// List unpushed commits between HEAD and a remote tracking branch.
///
/// Returns the `git log --oneline --decorate` output as a string, or an empty
/// string if no unpushed commits exist.
pub fn list_unpushed_commits(remote: &str, branch: &str, work_dir: &Path) -> Result<String> {
    let remote_ref = format!("remotes/{}/{}", remote, branch);
    let output = git_output(
        &[
            "log",
            "--oneline",
            "--decorate",
            "HEAD",
            "--not",
            &remote_ref,
        ],
        work_dir,
    );

    match output {
        Ok(text) => Ok(text),
        Err(_) => {
            // Remote branch may not exist yet; show all commits
            git_output(&["log", "--oneline", "--decorate"], work_dir)
        }
    }
}

/// Fetch a specific ref from a remote.
pub fn git_fetch_ref(remote: &str, git_ref: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["fetch", remote, git_ref], work_dir)
}

/// Create and checkout a new branch at the given start point.
pub fn git_checkout_new_branch(branch: &str, start_point: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["checkout", "-b", branch, start_point], work_dir)
}

/// Cherry-pick a commit onto the current branch.
pub fn git_cherry_pick(commit: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["cherry-pick", commit], work_dir)
}

/// Cherry-pick with "(cherry picked from commit ...)" indication.
pub fn git_cherry_pick_indicate(commit: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["cherry-pick", "-x", commit], work_dir)
}

/// Cherry-pick without committing (apply to working directory only).
pub fn git_cherry_pick_no_commit(commit: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["cherry-pick", "--no-commit", commit], work_dir)
}

/// Fill credentials from git credential helper.
///
/// Returns `Ok(Some((username, password)))` if credentials were found,
/// `Ok(None)` if the credential helper failed or did not return both fields.
///
/// Note: We send protocol= and host= fields separately. The original git-review
/// sends url=<full_url> instead. Both formats are valid per git-credential(1),
/// but some credential helpers may behave differently.
pub fn git_credential_fill(url: &str, work_dir: &Path) -> Result<Option<(String, String)>> {
    use std::process::Stdio;

    let parsed = url::Url::parse(url).context("parsing URL for credential fill")?;
    let input = format!(
        "protocol={}\nhost={}\n",
        parsed.scheme(),
        parsed.host_str().unwrap_or("")
    );

    let mut child = Command::new("git")
        .args(["credential", "fill"])
        .current_dir(work_dir)
        .env("LANG", "C")
        .env("LANGUAGE", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning git credential fill")?;

    use std::io::Write;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .context("writing to git credential fill")?;
    }

    let output = child
        .wait_with_output()
        .context("waiting for git credential fill")?;

    if !output.status.success() {
        return Ok(None);
    }

    let stdout =
        String::from_utf8(output.stdout).context("credential output is not valid UTF-8")?;

    let mut username = None;
    let mut password = None;
    for line in stdout.lines() {
        if let Some(val) = line.strip_prefix("username=") {
            username = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("password=") {
            password = Some(val.to_string());
        }
    }

    match (username, password) {
        (Some(u), Some(p)) => Ok(Some((u, p))),
        _ => Ok(None),
    }
}

/// Approve credentials with git credential helper (call after successful auth).
pub fn git_credential_approve(
    url: &str,
    username: &str,
    password: &str,
    work_dir: &Path,
) -> Result<()> {
    use std::process::Stdio;

    let parsed = url::Url::parse(url).context("parsing URL for credential approve")?;
    let input = format!(
        "protocol={}\nhost={}\nusername={}\npassword={}\n",
        parsed.scheme(),
        parsed.host_str().unwrap_or(""),
        username,
        password,
    );

    let mut child = Command::new("git")
        .args(["credential", "approve"])
        .current_dir(work_dir)
        .env("LANG", "C")
        .env("LANGUAGE", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawning git credential approve")?;

    use std::io::Write;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }

    let _ = child.wait();
    Ok(())
}

/// Reject credentials with git credential helper (call after auth failure).
pub fn git_credential_reject(
    url: &str,
    username: &str,
    password: &str,
    work_dir: &Path,
) -> Result<()> {
    use std::process::Stdio;

    let parsed = url::Url::parse(url).context("parsing URL for credential reject")?;
    let input = format!(
        "protocol={}\nhost={}\nusername={}\npassword={}\n",
        parsed.scheme(),
        parsed.host_str().unwrap_or(""),
        username,
        password,
    );

    let mut child = Command::new("git")
        .args(["credential", "reject"])
        .current_dir(work_dir)
        .env("LANG", "C")
        .env("LANGUAGE", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawning git credential reject")?;

    use std::io::Write;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }

    let _ = child.wait();
    Ok(())
}

/// Checkout an existing branch.
pub fn git_checkout(branch: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["checkout", branch], work_dir)
}

/// Delete a local branch.
pub fn git_delete_branch(branch: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["branch", "-D", branch], work_dir)
}

/// Run `git remote update <remote>` to fetch latest refs.
pub fn git_remote_update(remote: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["remote", "update", remote], work_dir)
}

/// Check if the working tree is clean (no staged or unstaged changes, ignoring submodules).
pub fn check_worktree_clean(work_dir: &Path) -> Result<bool> {
    let unstaged = git_command(&["diff", "--ignore-submodules", "--quiet"], work_dir)
        .status()
        .context("checking for unstaged changes")?;
    if !unstaged.success() {
        return Ok(false);
    }

    let staged = git_command(
        &["diff", "--cached", "--ignore-submodules", "--quiet"],
        work_dir,
    )
    .status()
    .context("checking for staged changes")?;
    Ok(staged.success())
}

/// Check if a remote tracking branch exists.
pub fn check_remote_branch_exists(remote: &str, branch: &str, work_dir: &Path) -> bool {
    let refname = format!("refs/remotes/{remote}/{branch}");
    git_command(&["show-ref", "--quiet", "--verify", &refname], work_dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Return the SHA of HEAD.
pub fn git_rev_parse_head(work_dir: &Path) -> Result<String> {
    git_output(&["rev-parse", "HEAD"], work_dir)
}

/// Rebase the current branch onto a remote branch.
///
/// Uses `--rebase-merges` and sets `GIT_EDITOR=true` to avoid interactive prompts.
pub fn git_rebase(remote_branch: &str, work_dir: &Path) -> Result<()> {
    let status = git_command(&["rebase", "--rebase-merges", remote_branch], work_dir)
        .env("GIT_EDITOR", "true")
        .status()
        .context("running git rebase")?;

    if !status.success() {
        anyhow::bail!(
            "git rebase {} failed (exit {})",
            remote_branch,
            status.code().unwrap_or(-1)
        );
    }
    Ok(())
}

/// Abort an in-progress rebase.
pub fn git_rebase_abort(work_dir: &Path) -> Result<()> {
    git_exec(&["rebase", "--abort"], work_dir)
}

/// Hard-reset to a specific commit.
pub fn git_reset_hard(commit: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["reset", "--hard", commit], work_dir)
}

/// Strip the Change-Id from HEAD and amend the commit.
///
/// The commit-msg hook will generate a new Change-Id on amend.
pub fn git_regenerate_changeid(work_dir: &Path) -> Result<()> {
    let msg = git_output(&["log", "-1", "--format=%B"], work_dir)?;
    let new_msg: String = msg
        .lines()
        .filter(|line| !line.starts_with("Change-Id:"))
        .collect::<Vec<_>>()
        .join("\n");
    git_exec(&["commit", "--amend", "-m", &new_msg], work_dir)
}

/// Fetch a ref from a remote and return the SHA it resolves to.
pub fn git_fetch_ref_sha(remote: &str, git_ref: &str, work_dir: &Path) -> Result<String> {
    git_exec(&["fetch", remote, git_ref], work_dir)?;
    git_output(&["rev-parse", "FETCH_HEAD"], work_dir)
}

/// Diff two commits, inheriting stdout/stderr for interactive output.
pub fn git_diff(commit_a: &str, commit_b: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["diff", commit_a, commit_b], work_dir)
}

/// Return the full `git config --list` output for URL rewrite parsing.
pub fn git_config_list(work_dir: &Path) -> Result<String> {
    git_output(&["config", "--list"], work_dir)
}

/// Set the upstream tracking branch for a local branch.
pub fn git_set_upstream_tracking(branch: &str, upstream: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["branch", "--set-upstream-to", upstream, branch], work_dir)
}

/// Checkout a branch, creating it if needed. If the branch already exists,
/// checks it out and resets to the start point (preserving working tree changes).
///
/// This mirrors git-review's behavior: try `checkout -b`, and on failure
/// fall back to `checkout` + `reset --keep`.
pub fn git_checkout_or_reset_branch(
    branch: &str,
    start_point: &str,
    work_dir: &Path,
) -> Result<()> {
    match git_exec(&["checkout", "-b", branch, start_point], work_dir) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Branch already exists — check it out and reset to start point
            git_exec(&["checkout", branch], work_dir).context("checking out existing branch")?;
            git_exec(&["reset", "--keep", start_point], work_dir)
                .context("resetting branch to new start point")
        }
    }
}

/// Add a new remote and fetch its refs.
pub fn git_remote_add(remote: &str, url: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["remote", "add", "-f", remote, url], work_dir)
}

/// Set the push URL for a remote.
pub fn git_remote_set_push_url(remote: &str, url: &str, work_dir: &Path) -> Result<()> {
    git_exec(&["remote", "set-url", "--push", remote, url], work_dir)
}

/// Check if a git remote exists and return its URL.
///
/// Returns `Ok(Some(url))` if the remote exists, `Ok(None)` if it doesn't.
pub fn check_remote_exists(remote: &str, work_dir: &Path) -> Result<Option<String>> {
    match git_output(&["remote", "get-url", remote], work_dir) {
        Ok(url) => Ok(Some(url)),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_output_success() {
        let dir = tempfile::tempdir().unwrap();
        let result = git_output(&["--version"], dir.path());
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.starts_with("git version"), "unexpected: {output}");
    }

    #[test]
    fn git_output_failure() {
        let dir = tempfile::tempdir().unwrap();
        let result = git_output(&["log", "--invalid-flag-that-does-not-exist"], dir.path());
        assert!(result.is_err());
    }
}
