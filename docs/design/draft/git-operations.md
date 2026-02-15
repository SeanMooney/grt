# grt Git Operations

**Related ref-specs:** `../ref-specs/git-review-workflow.md`
**Source code:** `crates/grt/src/git.rs`, `crates/grt/src/subprocess.rs`, `crates/grt/src/hook.rs`, `crates/grt/src/push.rs`, `crates/grt/src/review.rs`
**Status:** Draft

## Overview

grt uses a dual approach for git operations:

- **gix** (gitoxide, pure-Rust git) for read-only repository queries: discovering the repo, reading HEAD, resolving config values, finding the hooks directory, resolving upstream tracking branches. These operations benefit from gix's structured API and avoid process spawning overhead.
- **Subprocess** (`std::process::Command`) for operations that modify state or interact with external systems: `git push`, `git fetch`, `git checkout`, `git cherry-pick`, `git diff`, `git log`, `git credential fill/approve/reject`, `git remote update`. These require the full git CLI behavior, particularly for Gerrit's custom receive-pack options.

The split is pragmatic: gix provides fast, type-safe access to repository metadata, while subprocess calls handle the cases where gix's API is incomplete or where Gerrit-specific behaviors require the canonical git implementation.

## Repository Discovery

### `GitRepo::open(path)`

Uses `gix::discover(path)` to find the nearest git repository at or above the given path. Handles standard repositories, repositories in parent directories, and bare repositories (detected but rejected by `root()` since grt requires a worktree).

### Worktree Root

`GitRepo::root()` returns the worktree path. Returns an error for bare repositories. The root path is used as the working directory for all subprocess calls and as the base for locating `.gitreview`.

## Branch and Commit Operations

### `current_branch()` -- Current Branch Name

Reads the symbolic HEAD ref via gix and strips the `refs/heads/` prefix. Returns an error if HEAD is detached (no symbolic ref). Used during push to determine the default branch and for informational display.

### `head_commit_message()` -- HEAD Commit Message

Reads the commit message of the HEAD commit via gix. Used to extract the Change-Id trailer for push validation and for auto-detecting the change identifier in `grt comments`.

### `config_value(key)` -- Git Config Lookup

Reads a single git config value via gix's config snapshot. Used for:
1. Loading `gitreview.*` config values during config layering
2. Resolving `core.hooksPath` for hook installation
3. Resolving `branch.<name>.remote/merge` for upstream tracking

### `upstream_branch()` -- Upstream Tracking Branch

Reads `branch.<name>.remote` and `branch.<name>.merge` from git config to determine the upstream tracking branch:

```rust
pub fn upstream_branch(&self) -> Result<Option<(String, String)>> {
    let branch = self.current_branch()?;
    let remote_key = format!("branch.{branch}.remote");
    let merge_key = format!("branch.{branch}.merge");
    let remote = match self.config_value(&remote_key) {
        Some(r) => r,
        None => return Ok(None),
    };
    let merge = match self.config_value(&merge_key) {
        Some(m) => m.strip_prefix("refs/heads/").unwrap_or(&m).to_string(),
        None => return Ok(None),
    };
    Ok(Some((remote, merge)))
}
```

Returns `Some((remote, branch))` if both are configured, `None` otherwise. Used by `--track` to resolve the push target when no explicit branch is provided.

### `is_dirty()` -- Working Tree Status

Checks for uncommitted changes by running `git status --porcelain` as a subprocess rather than gix, since gix's status API requires careful feature flag management.

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
| reviewers | `r=<user>` | One per reviewer; whitespace rejected |
| cc | `cc=<user>` | One per CC recipient |
| hashtags | `hashtag=<tag>` | One per hashtag |
| message | `m=<url-encoded-text>` | URL-encoded via `urlencoding::encode()` |
| notify | `notify=<setting>` | NONE, OWNER, OWNER_REVIEWERS, ALL |
| no-rebase | `submit=false` | Disable automatic rebase |

### Change-Id Extraction and Validation

`push::extract_change_id()` scans the commit message from bottom to top for a `Change-Id:` trailer. Validation is strict: the ID must start with `I`, be exactly 41 characters, and the remaining 40 characters must be hexadecimal.

### Pre-Push Operations

Several flags trigger operations before the push:

- **`--update`** (`-u`): Runs `git remote update <remote>` to fetch latest refs
- **`--new-changeid`** (`-i`): Strips the existing Change-Id trailer and amends the commit (the commit-msg hook generates a new one)
- **`--track`**: Resolves the upstream tracking branch via `upstream_branch()` and uses it as the push target

### Post-Push Operations

- **`--finish`** (`-f`): After a successful push, checks out the target branch and deletes the current topic branch

### Multi-Commit Confirmation

Before pushing, `cmd_push` counts unpushed commits and prompts for confirmation when there are multiple (unless `--yes` is set).

## Download Workflow

### `cmd_review_download()` -- Download a Change

Downloads a change from Gerrit by fetching the patchset ref and creating a local branch.

**Workflow:**
1. Normalize change argument (parse URL patterns into `CHANGE[,PS]` format)
2. Split into change ID and optional patchset number
3. Authenticate and verify credentials
4. Fetch change detail with `ALL_REVISIONS`
5. Find the target revision (specific patchset or current)
6. `git fetch <remote> <ref>` to retrieve the patchset
7. `git checkout -b <branch> FETCH_HEAD` to create the local branch

**Branch naming:** `review/<owner>/<topic>` when both owner username and topic are available, otherwise `review/<change_number>/<patchset>`.

### URL Parsing

The `parse_change_url()` function normalizes Gerrit URLs to `CHANGE[,PS]` format:

| URL Pattern | Example | Result |
|------------|---------|--------|
| Simple | `https://review.example.com/12345` | `"12345"` |
| With patchset | `https://review.example.com/12345/2` | `"12345,2"` |
| Fragment | `https://review.example.com/#/c/12345` | `"12345"` |
| PolyGerrit | `https://review.example.com/c/project/+/12345/1` | `"12345,1"` |

## Cherry-Pick Workflow

Three modes for applying a Gerrit change to the current branch:

### `cmd_review_cherrypick()` -- Standard Cherry-Pick (`-x`)

Fetches the patchset ref and runs `git cherry-pick FETCH_HEAD`.

### `cmd_review_cherrypickindicate()` -- With Indication (`-X`)

Fetches the ref and runs `git cherry-pick -x FETCH_HEAD`, adding "(cherry picked from commit ...)" to the commit message.

### `cmd_review_cherrypickonly()` -- No Commit (`-N`)

Fetches the ref and runs `git cherry-pick --no-commit FETCH_HEAD`, applying changes to the working directory without creating a commit.

## Compare Workflow

### `cmd_review_compare()` -- Diff Patchsets (`-m`)

Compares two patchsets of a change by fetching both refs and running `git diff`.

**Workflow:**
1. Parse compare argument (`CHANGE,PS[-PS]`)
2. Authenticate
3. Fetch change detail with `ALL_REVISIONS`
4. Find both target revisions
5. `git fetch <remote> <ref>` for each patchset, capturing the resolved SHA via `git rev-parse FETCH_HEAD`
6. `git diff <sha1> <sha2>` with inherited stdout for interactive output

## Hook Management

### Vendored commit-msg Hook

The Gerrit commit-msg hook is embedded directly in the grt binary using `include_str!`. This eliminates the need for network access during hook installation.

### `ensure_hook_installed(hooks_dir)`

Installs the hook if not already present. Key behaviors:

- **Idempotent**: Returns `Ok(())` immediately if the hook file exists
- **Creates directories**: If the hooks directory doesn't exist, `create_dir_all` creates it
- **Executable permissions**: On Unix, sets `0o755`
- **Force reinstall**: `grt setup --force-hook` removes the existing hook before calling `ensure_hook_installed`

### Hooks Directory Resolution

`GitRepo::hooks_dir()` respects `core.hooksPath` git config:

- Absolute path: used as-is
- Relative path: resolved against the worktree root
- Not set: defaults to `<git_dir>/hooks`

## Subprocess Operations

### Core Helpers

| Function | Purpose | Output |
|----------|---------|--------|
| `git_output(args, work_dir)` | Capture stdout | `Result<String>` |
| `git_exec(args, work_dir)` | Interactive (inherited stdout/stderr) | `Result<()>` |

### Download/Cherry-Pick Operations

| Function | Git Command | Used By |
|----------|-------------|---------|
| `git_fetch_ref(remote, ref, dir)` | `git fetch <remote> <ref>` | Download, cherry-pick |
| `git_checkout_new_branch(branch, start, dir)` | `git checkout -b <branch> <start>` | Download |
| `git_cherry_pick(commit, dir)` | `git cherry-pick <commit>` | Cherry-pick (`-x`) |
| `git_cherry_pick_indicate(commit, dir)` | `git cherry-pick -x <commit>` | Cherry-pick indicate (`-X`) |
| `git_cherry_pick_no_commit(commit, dir)` | `git cherry-pick --no-commit <commit>` | Cherry-pick only (`-N`) |

### Compare Operations

| Function | Git Command | Used By |
|----------|-------------|---------|
| `git_fetch_ref_sha(remote, ref, dir)` | `git fetch` + `git rev-parse FETCH_HEAD` | Compare |
| `git_diff(sha1, sha2, dir)` | `git diff <sha1> <sha2>` | Compare |

### Push-Related Operations

| Function | Git Command | Used By |
|----------|-------------|---------|
| `git_checkout(branch, dir)` | `git checkout <branch>` | `--finish` |
| `git_delete_branch(branch, dir)` | `git branch -D <branch>` | `--finish` |
| `git_remote_update(remote, dir)` | `git remote update <remote>` | `--update` |
| `git_regenerate_changeid(dir)` | Strip Change-Id + `git commit --amend` | `--new-changeid` |
| `count_unpushed_commits(remote, branch, dir)` | `git log HEAD --not remotes/<remote>/<branch> --oneline` | Push confirmation |

### Credential Operations

| Function | Git Command | Used By |
|----------|-------------|---------|
| `git_credential_fill(url, dir)` | `git credential fill` | Credential fallback |
| `git_credential_approve(url, user, pass, dir)` | `git credential approve` | After successful auth |
| `git_credential_reject(url, user, pass, dir)` | `git credential reject` | After failed auth |

## Future Work

### NoteDb Reading

Not yet implemented. Gerrit stores metadata in special git refs. Reading NoteDb would enable offline access to change metadata without REST API calls.

### gix Push Support

Currently, `git push` is always done via subprocess because gix's push support may not handle Gerrit's custom receive-pack options (the `%topic=...` syntax in refspecs).

## Divergences from git-review (`git-review-workflow.md`)

- **gix for reads, subprocess for writes**: git-review calls all git operations via subprocess. grt uses gix's structured API for read operations.
- **No rebase workflow**: git-review's default "test rebase then undo" is not implemented.
- **No auto-amend for missing Change-Id**: git-review amends the commit if the hook wasn't installed. grt returns an error.
- **No remote creation**: git-review creates the Gerrit remote if it doesn't exist. grt assumes it exists.
- **No submodule hook propagation**: git-review copies the commit-msg hook into submodules. grt installs it only in the main repository.
- **Hook fetch**: git-review supports fetching the hook via HTTP or SCP. grt supports both when using `--remote-hook`; the protocol (HTTP vs SCP) is chosen from the resolved remote URL, matching git-review.
- **Strict Change-Id validation**: git-review checks for any `Change-Id:` prefix. grt validates the full format (I + 40 hex characters).
- **URL parsing**: git-review does not parse Gerrit URLs for download. grt supports multiple URL formats for change argument normalization.
