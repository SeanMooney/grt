# grt Git Operations

**Related ref-specs:** `../ref-specs/git-review-workflow.md`
**Source code:** `crates/grt/src/git.rs`, `crates/grt/src/subprocess.rs`, `crates/grt/src/hook.rs`, `crates/grt/src/push.rs`
**Status:** Draft

## Overview

grt uses a dual approach for git operations:

- **gix** (gitoxide, pure-Rust git) for read-only repository queries: discovering the repo, reading HEAD, resolving config values, finding the hooks directory. These operations benefit from gix's structured API and avoid process spawning overhead.
- **Subprocess** (`std::process::Command`) for operations that modify state or interact with external systems: `git push`, `git log`, `git credential fill/approve/reject`. These require the full git CLI behavior, particularly for Gerrit's custom receive-pack options.

The split is pragmatic: gix provides fast, type-safe access to repository metadata, while subprocess calls handle the cases where gix's API is incomplete or where Gerrit-specific behaviors require the canonical git implementation.

## Repository Discovery

### `GitRepo::open(path)`

Uses `gix::discover(path)` to find the nearest git repository at or above the given path. This handles:

- Standard repositories (`.git` directory)
- Repositories in parent directories (walking up the directory tree)
- Bare repositories (detected, but rejected by `root()` since grt requires a worktree)

```rust
pub fn open(path: &Path) -> Result<Self> {
    let repo = gix::discover(path).context("discovering git repository")?;
    Ok(Self { repo })
}
```

### Worktree Root

`GitRepo::root()` returns the worktree path. Returns an error for bare repositories:

```rust
pub fn root(&self) -> Result<PathBuf> {
    self.repo
        .workdir()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("repository is bare (no worktree)"))
}
```

The root path is used as the working directory for all subprocess calls and as the base for locating `.gitreview`.

## Branch and Commit Operations

### `current_branch()` -- Current Branch Name

Reads the symbolic HEAD ref via gix and strips the `refs/heads/` prefix:

```rust
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
```

Returns an error if HEAD is detached (no symbolic ref). Used during push to determine the default branch and for informational display.

### `head_commit_message()` -- HEAD Commit Message

Reads the commit message of the HEAD commit via gix:

```rust
pub fn head_commit_message(&self) -> Result<String> {
    let head = self.repo.head_commit().context("reading HEAD commit")?;
    let message = head.message_raw().context("reading commit message")?;
    Ok(message.to_string())
}
```

Used to extract the Change-Id trailer for push validation and for auto-detecting the change identifier in `grt comments`.

### `config_value(key)` -- Git Config Lookup

Reads a single git config value via gix's config snapshot:

```rust
pub fn config_value(&self, key: &str) -> Option<String> {
    let config = self.repo.config_snapshot();
    config.string(key).map(|v| v.to_string())
}
```

Used for two purposes:
1. Loading `gitreview.*` config values during config layering
2. Resolving `core.hooksPath` for hook installation

### `is_dirty()` -- Working Tree Status

Checks for uncommitted changes by running `git status --porcelain` as a subprocess:

```rust
pub fn is_dirty(&self) -> Result<bool> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(self.root()?)
        .output()
        .context("running git status")?;
    Ok(!output.stdout.is_empty())
}
```

This uses a subprocess rather than gix because gix's status API requires careful feature flag management. Not currently used in the MVP workflow but available for future commands.

### `count_unpushed_commits()` -- Unpushed Commit Count

Counts commits between HEAD and the remote tracking branch via subprocess:

```rust
pub fn count_unpushed_commits(remote: &str, branch: &str, work_dir: &Path) -> Result<usize> {
    let remote_ref = format!("remotes/{}/{}", remote, branch);
    let output = git_output(
        &["log", "HEAD", "--not", &remote_ref, "--oneline"],
        work_dir,
    );
    // ...
}
```

When the remote tracking branch doesn't exist (first push to a new branch), falls back to counting all commits via `git log --oneline`. Returns 0 when there are no unpushed commits.

This is the same approach git-review uses (`git log HEAD --not --remotes=<remote>`).

## Push Workflow

### Refspec Builder

`push::build_refspec()` constructs the Gerrit magic refspec from `PushOptions`:

```
HEAD:refs/for/<branch>[%<option1>,<option2>,...]
```

**Supported options:**

| Option | Format | Notes |
|--------|--------|-------|
| topic | `topic=<name>` | Skipped if same as branch name |
| wip | `wip` | Mark as work-in-progress |
| ready | `ready` | Mark as ready for review |
| private | `private` | Mark as private |
| remove-private | `remove-private` | Remove private flag |
| reviewers | `r=<user>` | One per reviewer; whitespace in names is rejected |
| cc | `cc=<user>` | One per CC recipient |
| hashtags | `hashtag=<tag>` | One per hashtag |
| message | `m=<url-encoded-text>` | URL-encoded via `urlencoding::encode()` |
| notify | `notify=<setting>` | NONE, OWNER, OWNER_REVIEWERS, ALL |
| no-rebase | `submit=false` | Disable automatic rebase |

Options are joined with commas and appended after a `%` separator. If no options are present, the refspec is just `HEAD:refs/for/<branch>`.

**Input validation:** Reviewer names containing whitespace are rejected with an error, since whitespace in the refspec would break the push command.

### Change-Id Extraction and Validation

`push::extract_change_id()` scans the commit message from bottom to top for a `Change-Id:` trailer:

```rust
pub fn extract_change_id(commit_message: &str) -> Option<String> {
    for line in commit_message.lines().rev() {
        let trimmed = line.trim();
        if let Some(id) = trimmed.strip_prefix("Change-Id: ") {
            let id = id.trim();
            if id.starts_with('I')
                && id.len() == 41
                && id[1..].chars().all(|c| c.is_ascii_hexdigit())
            {
                return Some(id.to_string());
            }
        }
    }
    None
}
```

The validation is strict: the ID must start with `I`, be exactly 41 characters, and the remaining 40 characters must be hexadecimal. This matches the format produced by Gerrit's commit-msg hook.

`push::validate_change_id()` wraps extraction with an actionable error message:

```
HEAD commit is missing a Change-Id trailer. Run `grt setup` to install the commit-msg hook, then amend the commit
```

### Multi-Commit Confirmation

Before pushing, `cmd_push` counts unpushed commits and prompts for confirmation when there are multiple:

```
About to push 3 commit(s) to gerrit/main. Continue? [y/N]
```

This matches git-review's behavior -- multiple commits create dependent changes in Gerrit, which is often unintentional for new users.

### Push Execution

The actual push uses `subprocess::git_exec()`, which inherits stdout/stderr for interactive output (allowing the user to see git's push progress and Gerrit's response):

```rust
subprocess::git_exec(&["push", &remote, &refspec], &root)?;
```

## Hook Management

### Vendored commit-msg Hook

The Gerrit commit-msg hook is embedded directly in the grt binary using `include_str!`:

```rust
const COMMIT_MSG_HOOK: &str = include_str!("../resources/commit-msg");
```

The vendored hook is sourced from Gerrit's official repository. This approach eliminates the need for network access during hook installation (no HTTP fetch or SCP), matching git-review's default behavior of writing a vendored copy.

### `ensure_hook_installed(hooks_dir)`

Installs the hook if not already present:

```rust
pub fn ensure_hook_installed(hooks_dir: &Path) -> Result<()> {
    let hook_path = hooks_dir.join("commit-msg");
    if hook_path.exists() {
        return Ok(());
    }
    // Create hooks directory if needed
    if !hooks_dir.exists() {
        std::fs::create_dir_all(hooks_dir).context("creating hooks directory")?;
    }
    std::fs::write(&hook_path, COMMIT_MSG_HOOK).context("writing commit-msg hook")?;
    // Set executable permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&hook_path, perms).context("setting hook permissions")?;
    }
    Ok(())
}
```

**Key behaviors:**

- **Idempotent**: Returns `Ok(())` immediately if the hook file exists, regardless of content. Existing hooks (including custom ones) are never overwritten.
- **Creates directories**: If the hooks directory doesn't exist (or is nested), `create_dir_all` creates it.
- **Executable permissions**: On Unix, sets `0o755` so git can execute the hook.
- **Force reinstall**: `grt setup --force-hook` removes the existing hook before calling `ensure_hook_installed`, allowing a fresh install.

### Hooks Directory Resolution

`GitRepo::hooks_dir()` respects the `core.hooksPath` git config:

```rust
pub fn hooks_dir(&self) -> Result<PathBuf> {
    if let Some(custom) = self.config_value("core.hooksPath") {
        let custom_path = Path::new(&custom);
        if custom_path.is_absolute() {
            return Ok(custom_path.to_path_buf());
        }
        let root = self.root()?;
        return Ok(root.join(custom_path));
    }
    let git_dir = self.repo.git_dir().to_path_buf();
    Ok(git_dir.join("hooks"))
}
```

- Absolute `core.hooksPath`: used as-is
- Relative `core.hooksPath`: resolved against the worktree root
- No `core.hooksPath`: defaults to `<git_dir>/hooks`

This matches git-review's `git_get_hooks_path()` behavior.

## Subprocess Operations

### `git_output(args, work_dir)` -- Capture Output

Runs a git command and captures stdout. Used for non-interactive operations where grt needs to parse the output:

```rust
pub fn git_output(args: &[&str], work_dir: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(work_dir)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {} failed (exit {}): {}", args.join(" "), ...);
    }
    Ok(stdout.trim_end().to_string())
}
```

**Used for:** `git log --oneline` (counting commits), `git remote get-url` (verifying remotes).

### `git_exec(args, work_dir)` -- Interactive Execution

Runs a git command with inherited stdout/stderr, allowing interactive output:

```rust
pub fn git_exec(args: &[&str], work_dir: &Path) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .current_dir(work_dir)
        .status()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !status.success() {
        anyhow::bail!("git {} failed (exit {})", ...);
    }
    Ok(())
}
```

**Used for:** `git push` (so the user sees Gerrit's push output).

### Credential Flow

Three subprocess functions manage git credentials:

**`git_credential_fill(url, work_dir)`**: Invokes `git credential fill` with the URL's protocol and host on stdin. Parses the key=value output to extract username and password.

**`git_credential_approve(url, username, password, work_dir)`**: Calls `git credential approve` to tell the credential helper to cache the credentials. Called after successful authentication.

**`git_credential_reject(url, username, password, work_dir)`**: Calls `git credential reject` to invalidate cached credentials. Called after authentication failure.

All three use piped stdin/stdout and silently ignore failures (the approve/reject operations are best-effort).

## Future Work

### NoteDb Reading

Not yet implemented. Gerrit stores metadata in special git refs:

- `refs/changes/XX/NNNN/meta` -- change metadata
- `refs/changes/XX/NNNN/N` -- patchset refs
- `refs/notes/review` -- review notes
- `refs/meta/config` -- project configuration

Reading NoteDb would enable offline access to change metadata without REST API calls. This would use gix to read ref contents and parse Gerrit's custom note format.

### Cherry-Pick

Not yet implemented. Planned as a `grt cherry-pick` command that applies a Gerrit change's commit to the local repository, similar to git-review's download/checkout workflow.

### gix Push Support

Currently, `git push` is always done via subprocess because gix's push support may not handle Gerrit's custom receive-pack options (the `%topic=...` syntax in refspecs). If gix adds support for custom refspec suffixes in the future, the subprocess call could be replaced.

## Divergences from git-review (`git-review-workflow.md`)

- **gix for reads, subprocess for writes**: git-review calls all git operations via subprocess and parses text output. grt uses gix's structured API for read operations, eliminating text parsing and LANG=C requirements.
- **No rebase workflow**: git-review's default "test rebase then undo" is not implemented. grt focuses on the push path and leaves rebasing to the user.
- **No auto-amend for missing Change-Id**: git-review amends the commit if the hook wasn't installed. grt returns an error and directs the user to run `grt setup`.
- **No remote creation**: git-review creates the Gerrit remote if it doesn't exist. grt assumes the remote exists.
- **No submodule hook propagation**: git-review copies the commit-msg hook into submodules. grt installs it only in the main repository.
- **Vendored hook only**: git-review supports fetching the hook via HTTP or SCP. grt always uses the vendored copy embedded in the binary.
- **Strict Change-Id validation**: git-review checks for any `Change-Id:` prefix. grt validates the full format (I + 40 hex characters).
