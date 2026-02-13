# grt CLI Design

**Related ref-specs:** `../ref-specs/git-review-workflow.md`
**Source code:** `crates/grt/src/main.rs`, `crates/grt/src/review.rs`, `crates/grt/src/export.rs`, `crates/grt/src/list.rs`
**Status:** Draft

## Overview

grt's CLI is built with clap's derive API. It supports two invocation modes:

1. **grt mode** (`argv[0]` is `grt`) -- uses subcommands: `review`, `push`, `comments`, `setup`, `export`, `version`, `completions`
2. **git-review mode** (`argv[0]` is `git-review`) -- flat flags matching git-review's exact CLI, compatible as a drop-in replacement

Both modes share the same `ReviewArgs` struct for the review/push workflow, ensuring behavioral consistency.

### Busybox-Style argv[0] Detection

The binary inspects `argv[0]` at startup to determine its personality:

```rust
enum Personality { Grt, GitReview }
```

When the binary basename is `git-review` (e.g., via symlink), it parses arguments as flat `ReviewArgs` (git-review compatible). When invoked as `grt`, it uses the `Cli` struct with subcommands.

The `grt export git-review` command creates a symlink at `~/.local/bin/git-review` pointing to the grt binary, enabling seamless git-review replacement.

## Command Tree

### `grt review [FLAGS] [BRANCH]` -- Git-Review Compatible Push/Download/List

The primary workflow command, mirroring git-review's exact flag set. When no mode flag is set, the default action is push. Mode flags are mutually exclusive.

**Positional arguments:**

| Argument | Description | Default |
|----------|-------------|---------|
| `branch` | Target branch for push, or filter for list | Config `defaultbranch` or `"main"` |

**Mode flags (mutually exclusive):**

| Flag | Short | Description |
|------|-------|-------------|
| `--download <CHANGE>` | `-d` | Download a change from Gerrit |
| `--cherrypick <CHANGE>` | `-x` | Cherry-pick a change onto current branch |
| `--cherrypickindicate <CHANGE>` | `-X` | Cherry-pick with "(cherry picked from ...)" indication |
| `--cherrypickonly <CHANGE>` | `-N` | Apply change to working directory without committing |
| `--compare <CHANGE,PS[-PS]>` | `-m` | Compare patchsets of a change |
| `--list` | `-l` | List open changes (`-l` brief, `-ll` verbose) |
| `--setup` | `-s` | Set up repository for Gerrit |

**Topic flags (mutually exclusive):**

| Flag | Short | Description |
|------|-------|-------------|
| `--topic <TOPIC>` | `-t` | Set the topic for the push |
| `--no-topic` | `-T` | Do not set a topic |

**Rebase flags (mutually exclusive):**

| Flag | Short | Description |
|------|-------|-------------|
| `--no-rebase` | `-R` | Do not rebase before pushing |
| `--force-rebase` | `-F` | Force rebase before pushing |

**Track flags (mutually exclusive):**

| Flag | Description |
|------|-------------|
| `--track` | Use upstream tracking branch as target |
| `--no-track` | Ignore upstream tracking branch |

**WIP flags (mutually exclusive):**

| Flag | Short | Description |
|------|-------|-------------|
| `--wip` / `--work-in-progress` | `-w` | Mark as work-in-progress |
| `--ready` | `-W` | Mark as ready for review |

**Privacy flags (mutually exclusive):**

| Flag | Short | Description |
|------|-------|-------------|
| `--private` | `-p` | Mark as private |
| `--remove-private` | `-P` | Remove private flag |

**Push metadata:**

| Flag | Description |
|------|-------------|
| `--reviewers <USER> [USER ...]` | Add reviewers |
| `--cc <USER> [USER ...]` | Add CC recipients |
| `--hashtags <TAG> [TAG ...]` | Add hashtags |
| `--notify <LEVEL>` | Notification setting (NONE, OWNER, OWNER_REVIEWERS, ALL) |
| `--message <TEXT>` | Review message |

**Behavior flags:**

| Flag | Short | Description |
|------|-------|-------------|
| `--remote <REMOTE>` | `-r` | Remote to push to |
| `--dry-run` | `-n` | Show what would be done without doing it |
| `--new-changeid` | `-i` | Generate a new Change-Id (amend HEAD) |
| `--yes` | `-y` | Skip confirmation prompts |
| `--update` | `-u` | Run `git remote update` before pushing |
| `--finish` | `-f` | Post-push cleanup: checkout default branch, delete topic branch |
| `--use-pushurl` | | Use the push URL instead of the fetch URL |
| `--no-thin` | | Disable thin pack for push |
| `--remote-hook` | | Execute a remote hook after push |
| `--no-custom-script` | | Do not run custom scripts |

#### Download Mode (`-d`)

Downloads a change from Gerrit by fetching the patchset ref and creating a local branch.

**Change argument formats:**
- Change number: `12345`
- Change number with patchset: `12345,2`
- Gerrit URL: `https://review.example.com/12345/2` (auto-parsed)
- PolyGerrit URL: `https://review.example.com/c/project/+/12345/1` (auto-parsed)
- Fragment URL: `https://review.example.com/#/c/12345` (auto-parsed)

**Branch naming:** `review/<owner>/<topic>` when both are available, otherwise `review/<change>/<patchset>`.

**Workflow:**
1. Normalize change argument (parse URL if needed)
2. Authenticate and verify credentials
3. Fetch change detail with `ALL_REVISIONS`
4. Find target revision (specific patchset or current)
5. `git fetch <remote> <ref>`
6. `git checkout -b <branch> FETCH_HEAD`

#### Cherry-pick Modes (`-x`, `-X`, `-N`)

All three modes fetch the patchset ref and apply it to the current branch:

- **`-x`** (`--cherrypick`): Standard cherry-pick (`git cherry-pick FETCH_HEAD`)
- **`-X`** (`--cherrypickindicate`): Cherry-pick with indication (`git cherry-pick -x FETCH_HEAD`)
- **`-N`** (`--cherrypickonly`): Apply without committing (`git cherry-pick --no-commit FETCH_HEAD`)

#### List Mode (`-l`, `-ll`)

Lists open changes from Gerrit.

**Query:** `status:open project:<project>` (plus `branch:<branch>` if branch specified).

**Brief output (`-l`):** Column-aligned with right-aligned change number, left-aligned branch, and subject.

**Verbose output (`-ll`):** Adds a topic column between branch and subject.

**Empty result:** Prints nothing (matches git-review).

#### Compare Mode (`-m`)

Compares two patchsets of a change by fetching both refs and running `git diff`.

**Argument format:** `CHANGE,PS[-PS]`
- `12345,1-3` -- diff patchset 1 against patchset 3
- `12345,1` -- diff patchset 1 against current revision

#### Track Mode (`--track`)

When `--track` is set and no explicit branch argument is provided, resolves the upstream tracking branch from `branch.<name>.remote` and `branch.<name>.merge` via gix config. Uses the merge branch as the push target.

#### Finish Mode (`--finish`)

After a successful push:
1. Checks out the target branch
2. Deletes the current topic branch

#### New Change-Id (`--new-changeid`)

Strips the existing `Change-Id:` trailer from the HEAD commit message and amends the commit. The commit-msg hook generates a new Change-Id during the amend.

#### Update (`--update`)

Runs `git remote update <remote>` before the push to fetch the latest refs.

### `grt push [branch]` -- Push Changes to Gerrit

Convenience alias that delegates to `grt review` in push mode. Retained for backwards compatibility.

### `grt comments [change]` -- Retrieve Review Comments

Fetches inline comments and review messages from Gerrit for a change, formatted for human reading or machine consumption.

**Positional arguments:**

| Argument | Description | Default |
|----------|-------------|---------|
| `change` | Change number or Change-Id | Auto-detected from HEAD commit's Change-Id trailer |

**Flags:**

| Flag | Description |
|------|-------------|
| `--revision <rev>` | Show comments for a specific patchset revision |
| `--unresolved` | Show only unresolved comment threads |
| `--format <fmt>` | Output format: `text` (default) or `json` |
| `--all-revisions` | Show comments from all revisions (not just current) |
| `--include-robot-comments` | Include automated/CI comments |

**Workflow:**

1. Load config and authenticate (credentials.toml or git credential helper)
2. Verify credentials via `GET /accounts/self`
3. Determine change identifier (from argument or HEAD commit)
4. Fetch change detail (for metadata and messages)
5. Fetch comments (scoped to current revision by default, or all/specific revision)
6. Optionally fetch and merge robot comments
7. Build comment threads (reply chain resolution via `in_reply_to` links)
8. Filter to unresolved if `--unresolved` set
9. Output in requested format

**Text output** is structured as LLM-friendly markdown with headers, review messages, inline comments grouped by file, and a summary with thread counts.

**JSON output** uses a structured `CommentOutput` schema with `change`, `review_messages`, `inline_comments`, and `summary` top-level keys.

### `grt setup [--remote <name>] [--force-hook]` -- Set Up Repository for Gerrit

Convenience alias that delegates to `grt review -s`. Verifies and configures the current repository for Gerrit usage.

### `grt export git-review [--clean]` -- Manage git-review Symlink

Creates or removes a symlink at `~/.local/bin/git-review` pointing to the current grt binary.

**Flags:**

| Flag | Description |
|------|-------------|
| `--clean` | Remove the symlink instead of creating it |

**Workflow:**
1. Resolve symlink path (`~/.local/bin/git-review`)
2. If `--clean`: remove the symlink if it exists
3. Otherwise: create directory if needed, resolve current exe, create symlink
4. Warn if `~/.local/bin` is not in PATH

### `grt version` -- Show Version Information

Prints grt's version and the Gerrit server version (if reachable).

### `grt completions <shell>` -- Generate Shell Completions

Generates shell completion scripts for bash, zsh, fish, elvish, or PowerShell using `clap_complete`.

```
grt completions bash > ~/.local/share/bash-completion/completions/grt
grt completions zsh > ~/.zfunc/_grt
grt completions fish > ~/.config/fish/completions/grt.fish
```

### Commands Not Yet Implemented

The following commands are planned for post-MVP:

- **`grt tui`** -- Launch the interactive terminal UI (ratatui)
- **`grt sync`** -- Trigger background sync
- **`grt search`** -- Fuzzy search across cached changes
- **`grt show`** -- Show detailed change info
- **`grt init`** -- Initialize a new repository with Gerrit config

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--verbose` | `-v` | Increase log verbosity (repeatable: `-v` info, `-vv` debug, `-vvv` trace) |
| `--directory <path>` | `-C` | Run as if started in `<path>` (like `git -C`) |
| `--no-color` | | Disable colored output |
| `--insecure` | | Allow sending credentials over plain HTTP (no TLS) |

All global flags use `global = true` in clap, making them available before or after the subcommand.

### Verbosity and Logging

The `-v` flag controls the tracing filter level for the `grt` target:

| Verbosity | Level | Shows |
|-----------|-------|-------|
| (none) | `warn` | Only warnings and errors |
| `-v` | `info` | Milestone events |
| `-vv` | `debug` | Flow and decision points |
| `-vvv` | `trace` | Data-level detail |

The `RUST_LOG` environment variable overrides the verbosity flag if set. Tracing output goes to stderr with no timestamps, suitable for CLI use.

## Output Formats

### Text (default)

Human-readable output to stdout. Informational messages (progress, status) go to stderr via `eprintln!()`, keeping stdout clean for data output.

### JSON

Machine-readable output via `--format json`. Currently only `grt comments` supports JSON output.

### List Output

Column-aligned tabular output for `grt review -l` and `grt review -ll`. Numbers are right-aligned, text columns are left-aligned with padding to the longest value. This matches git-review's output format.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Generic error |
| 2 | Hook-related error |
| 3 | Malformed input (bad argument format) |
| 40 | Network/connectivity error |
| 128 | Git config error (no Gerrit host configured) |

Exit codes are mapped from error types via `exit_code_for_error()`:
- `GerritError::Network` maps to exit code 40
- Errors containing "git config" or "no Gerrit host configured" map to 128
- Errors containing "argument" or "CHANGE,PS" map to 3
- Errors containing "hook" map to 2
- All other errors map to 1

This is compatible with git-review's exit code conventions.

## Interactive Behavior

### Multi-Commit Push Confirmation

When pushing more than one commit, grt prompts for confirmation (unless `--yes` is set):

```
About to push 3 commit(s) to gerrit/main. Continue? [y/N]
```

Only `y` (case-insensitive) proceeds; any other input cancels the push.

### No Commits Found

When no unpushed commits are detected, grt prints a message and exits cleanly (exit code 0):

```
No unpushed commits found.
```

## Divergences from git-review (`git-review-workflow.md`)

- **Subcommand syntax**: git-review uses `git review [branch]` (as a git subcommand). grt uses `grt review [branch]` (standalone binary with explicit subcommands), but also supports git-review-compatible flat syntax when invoked as `git-review` via symlink.
- **No rebase workflow**: git-review's default "test rebase then undo" is not implemented. grt's `--no-rebase` flag sends `submit=false` to Gerrit, which is a different mechanism.
- **No auto-amend**: git-review automatically amends the HEAD commit to add a Change-Id if the hook wasn't installed. grt validates the Change-Id and errors if missing, directing the user to run `grt setup`.
- **No remote creation**: git-review auto-creates the Gerrit remote if missing. grt requires the remote to exist (verified via `grt setup`).
- **No pre/post-review hooks**: git-review runs custom `pre-review` and `post-review` scripts. grt does not support custom hook scripts.
- **Comments command**: git-review has no comment retrieval feature. This is unique to grt.
- **Export command**: git-review has no equivalent of `grt export git-review` for managing symlinks.
