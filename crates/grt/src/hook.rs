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
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&hook_path, perms).context("setting hook permissions")?;
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
        assert_eq!(perms.mode() & 0o755, 0o755);
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
}
