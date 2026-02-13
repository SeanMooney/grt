# grt CLI Design

**Related ref-specs:** `../ref-specs/git-review-workflow.md`
**Source code:** `crates/grt/src/main.rs`
**Status:** Draft

## Overview

grt's CLI is built with clap's derive API. The `Cli` struct defines global flags, and the `Commands` enum defines subcommands. Each subcommand has its own args struct (`PushArgs`, `CommentsArgs`, `SetupArgs`). The `#[tokio::main]` entry point parses args, initializes tracing, and routes to the appropriate command handler.

The MVP implements four commands: `push`, `comments`, `setup`, and `version`. The CLI is designed for both interactive use (confirmation prompts, colored output) and scripting (JSON output, `--yes` to skip prompts, exit codes).

## Command Tree

### `grt push [branch]` -- Push Changes to Gerrit

Constructs and executes a `git push` command targeting Gerrit's magic `refs/for/<branch>` refspec. This is the primary workflow command.

**Positional arguments:**

| Argument | Description | Default |
|----------|-------------|---------|
| `branch` | Target branch for the push | Config `defaultbranch` or `"main"` |

**Flags:**

| Flag | Short | Description |
|------|-------|-------------|
| `--remote <name>` | | Remote to push to (default: config `defaultremote` or `"gerrit"`) |
| `--topic <name>` | | Topic for the change (skipped if same as branch) |
| `--wip` | | Mark as work-in-progress |
| `--ready` | | Mark as ready for review |
| `--private` | | Mark as private |
| `--remove-private` | | Remove private flag |
| `--reviewers <user>` | `-r` | Add reviewers (comma-separated or repeated) |
| `--cc <user>` | | Add CC recipients (comma-separated or repeated) |
| `--hashtags <tag>` | | Add hashtags (comma-separated or repeated) |
| `--message <text>` | `-m` | Review message (URL-encoded in refspec) |
| `--notify <setting>` | | Notification setting (NONE, OWNER, etc.) |
| `--no-rebase` | | Disable automatic rebase (sends `submit=false`) |
| `--dry-run` | | Show what would be pushed without pushing |
| `--yes` | `-y` | Skip confirmation prompt for multi-commit push |
| `--new-changeid` | | Generate a new Change-Id (amend HEAD) |

**Workflow:**

1. Load config and open repository via `App::new()`
2. Ensure commit-msg hook is installed (idempotent)
3. Validate that HEAD commit has a Change-Id trailer
4. Count unpushed commits via `git log HEAD --not remotes/<remote>/<branch>`
5. If zero unpushed: print message and exit
6. If multiple unpushed and `--yes` not set: prompt for confirmation
7. Build the refspec: `HEAD:refs/for/<branch>%<options>`
8. If `--dry-run`: print the `git push` command and exit
9. Execute `git push <remote> <refspec>` via subprocess

**Example refspecs produced:**

```
HEAD:refs/for/main
HEAD:refs/for/main%topic=my-feature,wip,r=alice,cc=bob
HEAD:refs/for/develop%topic=feature-x,wip,r=alice,cc=bob,hashtag=urgent
```

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

### `grt setup` -- Set Up Repository for Gerrit

Verifies and configures the current repository for Gerrit usage. Performs multiple checks and reports results.

**Flags:**

| Flag | Description |
|------|-------------|
| `--remote <name>` | Remote name to configure (default: config value) |
| `--force-hook` | Force reinstall of commit-msg hook even if it exists |

**Workflow:**

1. Install commit-msg hook (or skip if exists, unless `--force-hook`)
2. Verify the configured remote exists (via `git remote get-url`)
3. Test connectivity to Gerrit (via `GET /config/server/version`)
4. If connectivity fails, retry with authentication
5. Verify authentication (via `GET /accounts/self`)
6. Report each step's result to stderr

**Example output:**

```
Setting up grt for Gerrit...
  commit-msg hook: installed at .git/hooks/commit-msg
  remote 'gerrit': ssh://review.example.com:29418/project
  Gerrit host: review.example.com
  connectivity: OK (Gerrit 3.9.1)
  authenticated as: Alice Smith <alice@example.com>

Setup complete.
```

### `grt version` -- Show Version Information

Prints grt's version and the Gerrit server version (if reachable).

**No flags.**

**Output:**

```
grt 0.1.0
Gerrit 3.9.1
```

If the server is unreachable or the repository is not configured, the Gerrit version line shows `unavailable`.

### Commands Not Yet Implemented

The following commands from the original stub are planned for post-MVP:

- **`grt tui`** -- Launch the interactive terminal UI (ratatui)
- **`grt list`** -- List changes from local cache
- **`grt sync`** -- Trigger background sync
- **`grt search`** -- Fuzzy search across cached changes
- **`grt show`** -- Show detailed change info
- **`grt cherry-pick`** -- Apply a Gerrit change locally
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

The `RUST_LOG` environment variable overrides the verbosity flag if set. Tracing output goes to stderr with no timestamps, suitable for CLI use:

```rust
tracing_subscriber::fmt()
    .with_env_filter(/* ... */)
    .with_target(false)
    .without_time()
    .init();
```

## Output Formats

### Text (default)

Human-readable output to stdout. Informational messages (progress, status) go to stderr via `eprintln!()`, keeping stdout clean for data output.

For `grt comments`, the text format is structured markdown designed to be readable by both humans and LLMs:

```
# Change 12345 — Fix authentication timeout
# Project: myproject | Branch: main | Status: NEW
# Owner: Alice Smith <alice@example.com>
# URL: https://review.example.com/c/myproject/+/12345

## Review Messages
### Bob (Patchset 3) — 2025-02-10 14:30:00
Patch Set 3: Code-Review-1

## Inline Comments
### File: src/auth.rs
#### Line 42 [UNRESOLVED] (2 comments)
> **Bob** (PS3) — 2025-02-10 14:00:00
> This timeout should be configurable.

> **Alice** (PS4) — 2025-02-10 15:00:00
> Done, moved to config.

## Summary
- Total inline comment threads: 3
- Unresolved: 1
- Resolved: 2
```

### JSON

Machine-readable output via `--format json`. Currently only `grt comments` supports JSON output. The schema:

```json
{
  "change": {
    "number": 12345,
    "subject": "Fix authentication timeout",
    "project": "myproject",
    "branch": "main",
    "status": "NEW",
    "owner": "Alice Smith",
    "owner_email": "alice@example.com",
    "url": "https://review.example.com/c/myproject/+/12345"
  },
  "review_messages": [...],
  "inline_comments": [...],
  "summary": {
    "total_threads": 3,
    "unresolved": 1,
    "resolved": 2
  }
}
```

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (anyhow chain printed to stderr) |

The MVP does not define more granular exit codes. Error messages use anyhow's `{:#}` display format, which chains context messages:

```
Error: verifying credentials against Gerrit: Gerrit API error (401): Unauthorized
```

### Future Exit Codes

Specific exit codes for scripting (e.g., 2 for auth failure) are planned but not yet implemented. The architecture doc describes typed error enums that could map to distinct exit codes.

## Interactive Behavior

### Multi-Commit Push Confirmation

When pushing more than one commit, grt prompts for confirmation (unless `--yes` is set):

```
About to push 3 commit(s) to gerrit/main. Continue? [y/N]
```

Only `y` (case-insensitive) proceeds; any other input cancels the push. This follows git-review's pattern of warning users about multi-commit pushes, which create dependent changes in Gerrit.

### No Commits Found

When no unpushed commits are detected, grt prints a message and exits cleanly (exit code 0):

```
No unpushed commits found.
```

## Shell Completion

Not yet implemented. Planned for post-MVP using clap's `clap_complete` integration.

## Divergences from git-review (`git-review-workflow.md`)

- **No rebase workflow**: git-review's default "test rebase then undo" is not implemented. grt's `--no-rebase` flag sends `submit=false` to Gerrit, which is a different mechanism.
- **No auto-amend**: git-review automatically amends the HEAD commit to add a Change-Id if the hook wasn't installed. grt validates the Change-Id and errors if missing, directing the user to run `grt setup`.
- **No remote creation**: git-review auto-creates the Gerrit remote if missing. grt requires the remote to exist (verified via `grt setup`).
- **No pre/post-review hooks**: git-review runs custom `pre-review` and `post-review` scripts. grt does not support custom hook scripts.
- **Subcommand syntax**: git-review uses `git review [branch]` (as a git subcommand). grt uses `grt push [branch]` (standalone binary with explicit subcommands).
- **Comments command**: git-review has no comment retrieval feature. This is unique to grt.
