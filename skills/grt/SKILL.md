---
name: grt
description: >
  This skill should be used when the user asks to "list gerrit changes",
  "download a patchset", "read review comments", "push to gerrit",
  "address review feedback", "summarize review comments", or mentions
  Gerrit code review, Change-Id, patchsets, or review workflows.
  It operates via the grt CLI tool.
version: 0.1.0
---

# grt — Gerrit Code Review Skill

## Prerequisites

Before using any grt command:

1. **Check `.gitreview` exists** in the repo root. If missing, tell the user to create one or run `grt setup`.
2. **Check `grt` is in PATH** by running `which grt`. If missing, instruct the user to install it (see `docs/user/installation.md`).
3. **Verify Gerrit connectivity** — if any command exits with code 128, the repo is not configured for Gerrit. Run `grt setup` to fix.

## Core Workflows

### List open changes

```bash
grt review -l                     # Brief: change number, branch, subject
grt review -ll                    # Verbose: adds topic column
grt review -l <branch>            # Filter by target branch
grt review -l --format json       # JSON array of ChangeInfo objects
```

For programmatic use, prefer `--format json` — it outputs a JSON array of full ChangeInfo objects with number, branch, subject, topic, status, owner, etc.

### Read review comments

```bash
grt comments                           # Auto-detect change from HEAD's Change-Id
grt comments <CHANGE>                  # Specific change number
grt comments --format json             # Structured JSON for programmatic use
grt comments --format text             # Human-readable (default)
grt comments --unresolved              # Only unresolved threads
grt comments --all-revisions           # Comments from all patchsets
grt comments --include-robot-comments  # Include CI/automated comments
```

For programmatic parsing, always use `--format json`. See `references/comment-json-schema.md` for the output schema.

### Download a change

```bash
grt review -d <CHANGE>                   # Download latest patchset
grt review -d <CHANGE>,<PS>              # Download specific patchset
grt review -d <CHANGE> --format json     # JSON output with branch, patchset, upstream
```

Creates a branch named `review/<owner>/<topic>` or `review/<change>/<patchset>`.

With `--format json`, outputs a `DownloadResult` object: `{ "branch", "change_number", "patchset", "upstream" }`.

**Safety:** Always check `git status --porcelain` before downloading. If there are uncommitted changes, warn the user and ask whether to stash or abort.

### Cherry-pick a change

```bash
grt review -x <CHANGE>         # Standard cherry-pick
grt review -X <CHANGE>         # Cherry-pick with "(cherry picked from ...)" indication
grt review -N <CHANGE>         # Apply to working directory only, no commit
```

All three accept `<CHANGE>,<PS>` to target a specific patchset.

### Compare patchsets

```bash
grt review -m <CHANGE>,<PS_FROM>-<PS_TO>   # Diff two patchsets
grt review -m <CHANGE>,<PS_FROM>           # Diff PS_FROM against current revision
```

### Push to Gerrit

**Always dry-run first, then ask for user confirmation before the actual push.**

```bash
# Step 1: Preview
grt review --dry-run [branch]

# Step 2: Show dry-run output to user, ask for confirmation

# Step 3: Actual push (only after explicit user approval)
grt review --yes [branch]

# Step 4 (optional): Structured output
grt review --yes --format json [branch]
```

With `--format json`, push outputs a `PushResult` object: `{ "commits", "remote", "branch", "change_id", "refspec" }`.

Common push options:

```bash
grt review -t <topic>                    # Set topic
grt review --reviewers alice bob         # Add reviewers
grt review --cc alice                    # Add CC
grt review --wip                         # Mark work-in-progress
grt review --ready                       # Mark ready for review
grt review -f                            # Post-push cleanup (checkout default, delete branch)
```

## The Feedback Loop

When the user asks to "address review feedback", "fix review comments", or similar, follow the 6-phase feedback loop protocol. This is a structured workflow for fetching review comments, planning fixes, implementing them, and pushing an updated patchset.

**Summary of phases:**

| Phase | Action | Checkpoint? |
|-------|--------|-------------|
| 1. Fetch | `grt comments --format json --unresolved` | |
| 2. Summarize | Present comments grouped by file, classify each | |
| 3. Plan | Read source, propose fixes, get user approval | YES |
| 4. Implement | Edit code, run tests | |
| 5. Push | `git commit --amend`, dry-run, user confirms push | YES |
| 6. Verify | `grt comments --format json` to confirm new patchset | |

Phases 3 and 5 are mandatory checkpoints — never skip user approval at these points.

See `references/feedback-loop-protocol.md` for the detailed protocol.

## Tips for Programmatic Use

- **Extract Change-Id from HEAD**: `git log -1 --format=%B | grep 'Change-Id:'`
- **Get branch after download**: `git branch --show-current` (or parse the JSON output from `--format json`)
- **Construct change URL**: `https://<host>/c/<project>/+/<number>` -- host from `.gitreview`, project from `git config`
- **Preferred: use `--format json`** for list, download, and push commands to get structured output instead of parsing text

## Multi-Change Batch Workflow

When asked to "address all open feedback" or work across multiple changes:

1. `grt review -l --format json` to list open changes
2. Parse the JSON array to extract change numbers
3. Present the list to the user and ask which changes to address
4. Run the feedback loop (see below) for each selected change sequentially

## Safety Rules

1. **Never push without confirmation.** Always `--dry-run` first, show the output, and wait for explicit user approval.
2. **Never read or display credentials.** Do not read `credentials.toml`, `.netrc`, or any file containing passwords or tokens.
3. **Always check git status before download.** Run `git status --porcelain` and warn about uncommitted changes.
4. **Verify Change-Id before amending.** Before `git commit --amend`, confirm the HEAD commit has a `Change-Id:` trailer.
5. **No destructive git operations.** Never run `reset --hard`, `clean -f`, `push --force`, or `checkout .` without explicit user request.
6. **Review objectivity.** When summarizing review comments, present technical merits fairly. Do not dismiss reviewer concerns.
7. **Scope limits.** grt cannot submit changes, vote, or edit change metadata via API. Direct the user to the Gerrit web UI for those operations.

## Error Handling

| Exit Code | Meaning | Recovery |
|-----------|---------|----------|
| 0 | Success | — |
| 1 | Generic error (auth failed, not found, server error) | Check credentials and change number |
| 2 | Hook-related error | Run `grt setup --force-hook` |
| 3 | Malformed input (bad argument format) | Check argument syntax (e.g., `CHANGE,PS` format) |
| 40 | Network/connectivity error | Check network, VPN, Gerrit server status |
| 128 | Git config error (no Gerrit host configured) | Run `grt setup` to configure the repository |

## Command Reference

For the full flag reference with mutual exclusivity rules and all options, see `references/grt-commands.md`.
