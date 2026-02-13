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

/// Fetch a commit-msg hook from a remote URL.
///
/// Currently a stub — prints an informative message.
/// TODO: implement HTTP and SCP hook download (git-review cmd.py:395-440)
pub fn fetch_remote_hook(url: &str, hooks_dir: &Path) -> Result<()> {
    eprintln!(
        "Remote hook download not yet implemented. \
         Please manually download the hook from {} and place it in {}",
        url,
        hooks_dir.display()
    );
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

    #[test]
    fn fetch_remote_hook_stub_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_dir = dir.path().join("hooks");
        let result = fetch_remote_hook("https://example.com/hook", &hooks_dir);
        assert!(result.is_ok());
    }
}
