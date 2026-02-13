// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use std::path::Path;

use anyhow::{Context, Result};

const COMMIT_MSG_HOOK: &str = include_str!("../resources/commit-msg");

/// Ensure the Gerrit commit-msg hook is installed in the repository's hooks directory.
/// Does not overwrite an existing hook.
pub fn ensure_hook_installed(hooks_dir: &Path) -> Result<()> {
    let hook_path = hooks_dir.join("commit-msg");

    if hook_path.exists() {
        return Ok(());
    }

    // Create hooks directory if it doesn't exist
    if !hooks_dir.exists() {
        std::fs::create_dir_all(hooks_dir).context("creating hooks directory")?;
    }

    std::fs::write(&hook_path, COMMIT_MSG_HOOK).context("writing commit-msg hook")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Get current file permissions (affected by umask)
        let metadata = std::fs::metadata(&hook_path).context("reading hook file metadata")?;
        let mode = metadata.permissions().mode();
        // Add execute bits matching read bits: if user can read, user can execute, etc.
        let exec_bits = (mode & 0o444) >> 2; // shift read bits to execute position
        let new_mode = mode | exec_bits;
        let perms = std::fs::Permissions::from_mode(new_mode);
        std::fs::set_permissions(&hook_path, perms).context("setting hook permissions")?;
    }

    Ok(())
}

/// Propagate the commit-msg hook to all submodules recursively.
pub fn propagate_hook_to_submodules(work_dir: &Path) -> Result<()> {
    use std::process::Command;
    let output = Command::new("git")
        .args([
            "submodule",
            "foreach",
            "--recursive",
            "echo $toplevel/$sm_path",
        ])
        .current_dir(work_dir)
        .output()
        .context("listing submodules")?;

    if !output.status.success() {
        // No submodules or git submodule not available — not an error
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Entering") {
            continue;
        }
        let submodule_path = Path::new(line);
        let hooks_dir = submodule_path.join(".git").join("hooks");
        if let Err(e) = ensure_hook_installed(&hooks_dir) {
            tracing::warn!("failed to install hook in submodule {}: {}", line, e);
        }
    }

    Ok(())
}

/// Parse an SSH/SCP-style URL into (user@host, optional port, path).
///
/// Supports:
/// - `ssh://[user@]host[:port]/path`
/// - `user@host:path` (SCP syntax)
pub fn parse_ssh_url(url: &str) -> Result<(String, Option<u16>, String)> {
    // Try ssh:// URL form first
    if let Some(rest) = url.strip_prefix("ssh://") {
        let (userhost, path) = rest
            .split_once('/')
            .context("SSH URL has no path component")?;

        let (userhost, port) = if let Some((uh, port_str)) = userhost.rsplit_once(':') {
            let port: u16 = port_str.parse().context("parsing SSH port from URL")?;
            (uh.to_string(), Some(port))
        } else {
            (userhost.to_string(), None)
        };

        return Ok((userhost, port, format!("/{path}")));
    }

    // Try SCP-style: user@host:path
    if let Some((userhost, path)) = url.split_once(':') {
        if !path.starts_with("//") && userhost.contains('@') {
            return Ok((userhost.to_string(), None, path.to_string()));
        }
    }

    anyhow::bail!("cannot parse SSH URL: {url}");
}

/// Fetch a commit-msg hook from a remote Gerrit server.
///
/// Tries HTTP(S) download first (`<base_url>/tools/hooks/commit-msg`).
/// Falls back to SCP for SSH-based URLs.
pub async fn fetch_remote_hook(url: &str, hooks_dir: &Path) -> Result<()> {
    let hook_path = hooks_dir.join("commit-msg");

    // Create hooks directory if needed
    if !hooks_dir.exists() {
        std::fs::create_dir_all(hooks_dir).context("creating hooks directory")?;
    }

    if url.starts_with("http://") || url.starts_with("https://") {
        fetch_hook_http(url, &hook_path).await?;
    } else {
        fetch_hook_scp(url, &hook_path)?;
    }

    // Set executable permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&hook_path).context("reading hook file metadata")?;
        let mode = metadata.permissions().mode();
        let exec_bits = (mode & 0o444) >> 2;
        let new_mode = mode | exec_bits;
        let perms = std::fs::Permissions::from_mode(new_mode);
        std::fs::set_permissions(&hook_path, perms).context("setting hook permissions")?;
    }

    eprintln!("  commit-msg hook: downloaded to {}", hook_path.display());
    Ok(())
}

/// Download hook via HTTP(S).
async fn fetch_hook_http(base_url: &str, hook_path: &Path) -> Result<()> {
    let hook_url = format!("{}/tools/hooks/commit-msg", base_url.trim_end_matches('/'));
    tracing::info!("Downloading commit-msg hook from {hook_url}...");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("creating HTTP client for hook download")?;

    let response = client
        .get(&hook_url)
        .send()
        .await
        .context("downloading commit-msg hook")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "failed to download commit-msg hook: HTTP {}",
            response.status()
        );
    }

    let bytes = response
        .bytes()
        .await
        .context("reading hook response body")?;
    std::fs::write(hook_path, &bytes).context("writing downloaded commit-msg hook")?;

    Ok(())
}

/// Download hook via SCP.
fn fetch_hook_scp(url: &str, hook_path: &Path) -> Result<()> {
    let (userhost, port, _path) = parse_ssh_url(url)?;
    let source = format!("{userhost}:hooks/commit-msg");

    tracing::info!("Downloading commit-msg hook via SCP from {source}...");

    let mut cmd = std::process::Command::new("scp");
    // Use -O for legacy SCP protocol (better compatibility)
    cmd.arg("-O");
    if let Some(p) = port {
        cmd.args(["-P", &p.to_string()]);
    }
    cmd.arg(&source);
    cmd.arg(hook_path);

    let output = cmd.output().context("running scp to download hook")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("scp failed: {}", stderr.trim());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_hook_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_dir = dir.path().join("hooks");
        ensure_hook_installed(&hooks_dir).unwrap();
        assert!(hooks_dir.join("commit-msg").exists());
    }

    #[cfg(unix)]
    #[test]
    fn install_hook_is_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let hooks_dir = dir.path().join("hooks");
        ensure_hook_installed(&hooks_dir).unwrap();
        let perms = std::fs::metadata(hooks_dir.join("commit-msg"))
            .unwrap()
            .permissions();
        assert!(perms.mode() & 0o100 != 0, "hook should be user-executable");
    }

    #[test]
    fn install_hook_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_dir = dir.path().join("hooks");
        ensure_hook_installed(&hooks_dir).unwrap();
        // Second call should not error
        ensure_hook_installed(&hooks_dir).unwrap();
        assert!(hooks_dir.join("commit-msg").exists());
    }

    #[test]
    fn install_hook_does_not_overwrite_existing() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_dir = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();
        let hook_path = hooks_dir.join("commit-msg");
        std::fs::write(&hook_path, "#!/bin/sh\n# custom hook\n").unwrap();

        ensure_hook_installed(&hooks_dir).unwrap();

        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(
            content.contains("custom hook"),
            "existing hook should not be overwritten"
        );
    }

    #[test]
    fn install_hook_creates_hooks_dir() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_dir = dir.path().join("deep").join("nested").join("hooks");
        ensure_hook_installed(&hooks_dir).unwrap();
        assert!(hooks_dir.join("commit-msg").exists());
    }

    #[test]
    fn propagate_hook_no_submodules() {
        let dir = tempfile::tempdir().unwrap();
        // Initialize a git repo with no submodules
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .expect("git init failed");
        // Should succeed without error even with no submodules
        let result = propagate_hook_to_submodules(dir.path());
        assert!(result.is_ok());
    }

    // === parse_ssh_url tests ===

    #[test]
    fn parse_ssh_url_standard() {
        let (userhost, port, path) =
            parse_ssh_url("ssh://alice@review.example.com:29418/project").unwrap();
        assert_eq!(userhost, "alice@review.example.com");
        assert_eq!(port, Some(29418));
        assert_eq!(path, "/project");
    }

    #[test]
    fn parse_ssh_url_no_port() {
        let (userhost, port, path) =
            parse_ssh_url("ssh://alice@review.example.com/project").unwrap();
        assert_eq!(userhost, "alice@review.example.com");
        assert_eq!(port, None);
        assert_eq!(path, "/project");
    }

    #[test]
    fn parse_ssh_url_scp_style() {
        let (userhost, port, path) = parse_ssh_url("git@review.example.com:project/repo").unwrap();
        assert_eq!(userhost, "git@review.example.com");
        assert_eq!(port, None);
        assert_eq!(path, "project/repo");
    }

    #[test]
    fn parse_ssh_url_invalid() {
        let result = parse_ssh_url("https://example.com/project");
        assert!(result.is_err());
    }
}
