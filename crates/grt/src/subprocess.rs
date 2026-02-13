// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

/// Run a git command and return its stdout output.
pub fn git_output(args: &[&str], work_dir: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(work_dir)
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
    let status = Command::new("git")
        .args(args)
        .current_dir(work_dir)
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

/// Fill credentials from git credential helper.
pub fn git_credential_fill(url: &str, work_dir: &Path) -> Result<(String, String)> {
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
        anyhow::bail!("git credential fill failed");
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
        (Some(u), Some(p)) => Ok((u, p)),
        _ => anyhow::bail!("git credential fill did not return username/password"),
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
