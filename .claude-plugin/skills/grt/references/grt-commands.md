# grt Command Reference

Agent-optimized terse reference. For user-facing docs see `docs/user/cli-reference.md`.

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--verbose` | `-v` | Increase verbosity (`-v` info, `-vv` debug, `-vvv` trace) |
| `--directory <PATH>` | `-C` | Run as if started in PATH |
| `--no-color` | | Disable colored output |
| `--insecure` | | Allow credentials over plain HTTP |

## grt review

Default action (no mode flag): **push** to Gerrit.

**Positional:** `[branch]` — target branch for push, or filter for list.

### Mode Flags (mutually exclusive — pick one)

| Flag | Short | Argument | Action |
|------|-------|----------|--------|
| `--list` | `-l` | — | List open changes (`-l` brief, `-ll` verbose) |
| `--download` | `-d` | `CHANGE[,PS]` | Download change to local branch |
| `--cherrypick` | `-x` | `CHANGE[,PS]` | Cherry-pick onto current branch |
| `--cherrypickindicate` | `-X` | `CHANGE[,PS]` | Cherry-pick with indication |
| `--cherrypickonly` | `-N` | `CHANGE[,PS]` | Apply to workdir, no commit |
| `--compare` | `-m` | `CHANGE,PS[-PS]` | Diff patchsets |
| `--setup` | `-s` | — | Set up repo for Gerrit |

### Topic (mutually exclusive)

| Flag | Short | Description |
|------|-------|-------------|
| `--topic <TOPIC>` | `-t` | Set push topic |
| `--no-topic` | `-T` | Do not set a topic |

### Rebase (mutually exclusive)

| Flag | Short | Description |
|------|-------|-------------|
| `--no-rebase` | `-R` | Skip rebase before push |
| `--force-rebase` | `-F` | Force rebase before push |
| `--keep-rebase` | `-K` | Keep rebase state on failure (with `-F`) |

Note: `--keep-rebase` conflicts with `--no-rebase`.

### Track (mutually exclusive)

| Flag | Description |
|------|-------------|
| `--track` | Use upstream tracking branch as target |
| `--no-track` | Ignore upstream tracking branch |

### WIP (mutually exclusive)

| Flag | Short | Description |
|------|-------|-------------|
| `--wip` / `--work-in-progress` | `-w` | Mark as work-in-progress |
| `--ready` | `-W` | Mark as ready for review |

### Privacy (mutually exclusive)

| Flag | Short | Description |
|------|-------|-------------|
| `--private` | `-p` | Mark as private |
| `--remove-private` | `-P` | Remove private flag |

### Push Metadata

| Flag | Argument | Description |
|------|----------|-------------|
| `--reviewers` | `USER [USER ...]` | Add reviewers |
| `--cc` | `USER [USER ...]` | Add CC recipients |
| `--hashtags` | `TAG [TAG ...]` | Add hashtags |
| `--notify` | `NONE\|OWNER\|OWNER_REVIEWERS\|ALL` | Notification level |
| `--message` | `TEXT` | Review message |

### Behavior Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--remote <REMOTE>` | `-r` | Remote to push to |
| `--dry-run` | `-n` | Show what would be done |
| `--new-changeid` | `-i` | Generate new Change-Id (amend HEAD) |
| `--yes` | `-y` | Skip confirmation prompts |
| `--update` | `-u` | Run `git remote update` first |
| `--finish` | `-f` | Post-push: checkout default branch, delete topic branch |
| `--use-pushurl` | | Use push URL instead of fetch URL |
| `--no-thin` | | Disable thin pack |
| `--remote-hook` | | Execute remote hook after push |
| `--no-custom-script` | | Do not run custom scripts |
| `--format <FMT>` | | `text` (default) or `json` — structured output for list, download, push |

## grt push

Native push interface (subset of review flags).

**Positional:** `[branch]`

| Flag | Short | Description |
|------|-------|-------------|
| `--remote <REMOTE>` | | Remote to push to |
| `--topic <TOPIC>` | | Topic for the change |
| `--wip` | | Mark as WIP |
| `--ready` | | Mark as ready |
| `--private` | | Mark as private |
| `--remove-private` | | Remove private flag |
| `--reviewers <USER>[,USER...]` | `-r` | Add reviewers |
| `--cc <USER>[,USER...]` | | Add CC |
| `--hashtags <TAG>[,TAG...]` | | Add hashtags |
| `--message <TEXT>` | `-m` | Review message |
| `--notify <LEVEL>` | | Notification setting |
| `--no-rebase` | | Disable auto-rebase |
| `--force-rebase` | | Force rebase |
| `--keep-rebase` | | Keep rebase state on failure |
| `--dry-run` | | Preview only |
| `--yes` | `-y` | Skip confirmation |
| `--new-changeid` | | Generate new Change-Id |
| `--no-thin` | | Disable thin pack |
| `--format <FMT>` | | `text` (default) or `json` — structured push output |

## grt comments

**Positional:** `[change]` — change number or Change-Id (auto-detected from HEAD if omitted).

| Flag | Description |
|------|-------------|
| `--revision <REV>` | Patchset revision |
| `--unresolved` | Only unresolved threads |
| `--format <FMT>` | `text` (default) or `json` |
| `--all-revisions` | All patchsets |
| `--include-robot-comments` | Include CI comments |

## grt setup

| Flag | Description |
|------|-------------|
| `--remote <NAME>` | Remote name to configure |
| `--force-hook` | Force reinstall commit-msg hook |
| `--remote-hook` | Download hook from server |

## grt export git-review

| Flag | Description |
|------|-------------|
| `--clean` | Remove symlink instead of creating |

## grt version

Show grt and Gerrit server versions. No flags.

## grt completions

**Argument:** `bash` | `zsh` | `fish` | `elvish` | `powershell`

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Generic error (auth, not found, server error) |
| 2 | Hook error |
| 3 | Malformed input |
| 40 | Network/connectivity error |
| 128 | Git config error (no Gerrit host) |
