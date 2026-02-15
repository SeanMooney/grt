# CLI Reference

Complete reference for grt commands, flags, and exit codes.

## Invocation Modes

grt supports two invocation modes:

| Mode | How | Syntax |
|------|-----|--------|
| **grt** | Invoked as `grt` | Subcommands: `grt review`, `grt push`, `grt comments`, etc. |
| **git-review** | Invoked as `git-review` (via symlink) | Flat flags: `git-review -l`, `git-review -d 12345`, etc. |

When the binary basename is `git-review`, grt parses arguments as flat flags compatible with git-review. Use `grt export git-review` to create the symlink.

## Global Flags

Available before or after the subcommand:

| Flag | Short | Description |
|------|-------|-------------|
| `--verbose` | `-v` | Increase verbosity (repeatable: `-v` info, `-vv` debug, `-vvv` trace) |
| `--directory <PATH>` | `-C` | Run as if started in `<PATH>` (like `git -C`) |
| `--no-color` | | Disable colored output |
| `--insecure` | | Allow sending credentials over plain HTTP (no TLS) |

## Commands

### grt review

Push changes to Gerrit for review (git-review compatible). When no mode flag is set, the default action is **push**.

**Positional argument:** `[branch]` — target branch for push, or filter for list (defaults to config or `main`)

#### Mode flags (mutually exclusive)

| Flag | Short | Description |
|------|-------|-------------|
| `--download <CHANGE>` | `-d` | Download a change from Gerrit |
| `--cherrypick <CHANGE>` | `-x` | Cherry-pick a change onto current branch |
| `--cherrypickindicate <CHANGE>` | `-X` | Cherry-pick with "(cherry picked from ...)" indication |
| `--cherrypickonly <CHANGE>` | `-N` | Apply change to working directory without committing |
| `--compare <CHANGE,PS[-PS]>` | `-m` | Compare patchsets of a change |
| `--list` | `-l` | List open changes (`-l` brief, `-ll` verbose) |
| `--setup` | `-s` | Set up repository for Gerrit |

#### Topic (mutually exclusive)

| Flag | Short | Description |
|------|-------|-------------|
| `--topic <TOPIC>` | `-t` | Set the topic for the push |
| `--no-topic` | `-T` | Do not set a topic |

#### Rebase (mutually exclusive)

| Flag | Short | Description |
|------|-------|-------------|
| `--no-rebase` | `-R` | Do not rebase before pushing |
| `--force-rebase` | `-F` | Force rebase before pushing |
| `--keep-rebase` | `-K` | Keep rebase state on failure (used with `--force-rebase`) |

#### Track (mutually exclusive)

| Flag | Description |
|------|-------------|
| `--track` | Use upstream tracking branch as target |
| `--no-track` | Ignore upstream tracking branch |

#### WIP (mutually exclusive)

| Flag | Short | Description |
|------|-------|-------------|
| `--wip` / `--work-in-progress` | `-w` | Mark as work-in-progress |
| `--ready` | `-W` | Mark as ready for review |

#### Privacy (mutually exclusive)

| Flag | Short | Description |
|------|-------|-------------|
| `--private` | `-p` | Mark as private |
| `--remove-private` | `-P` | Remove private flag |

#### Push metadata

| Flag | Description |
|------|-------------|
| `--reviewers <USER> [USER ...]` | Add reviewers |
| `--cc <USER> [USER ...]` | Add CC recipients |
| `--hashtags <TAG> [TAG ...]` | Add hashtags |
| `--notify <LEVEL>` | Notification level: NONE, OWNER, OWNER_REVIEWERS, ALL |
| `--message <TEXT>` | Review message |

#### Behavior flags

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

### grt push

Push changes to Gerrit (native grt interface, subset of review flags).

**Positional argument:** `[branch]` — target branch (defaults to config or `main`)

| Flag | Short | Description |
|------|-------|-------------|
| `--remote <REMOTE>` | | Remote to push to |
| `--topic <TOPIC>` | | Topic for the change |
| `--wip` | | Mark as work-in-progress |
| `--ready` | | Mark as ready for review |
| `--private` | | Mark as private |
| `--remove-private` | | Remove private flag |
| `--reviewers <USER>[,USER...]` | `-r` | Add reviewers (comma-separated or repeated) |
| `--cc <USER>[,USER...]` | | Add CC recipients |
| `--hashtags <TAG>[,TAG...]` | | Add hashtags |
| `--message <TEXT>` | `-m` | Review message |
| `--notify <LEVEL>` | | Notification setting |
| `--no-rebase` | | Disable automatic rebase |
| `--force-rebase` | | Force rebase before pushing |
| `--keep-rebase` | | Keep rebase state on failure |
| `--dry-run` | | Show what would be pushed without pushing |
| `--yes` | `-y` | Skip confirmation prompt |
| `--new-changeid` | | Generate a new Change-Id |
| `--no-thin` | | Disable thin pack for push |

### grt comments

Retrieve review comments from Gerrit.

**Positional argument:** `[change]` — change number or Change-Id (auto-detected from HEAD if omitted)

| Flag | Description |
|------|-------------|
| `--revision <REV>` | Patchset revision to show comments for |
| `--unresolved` | Show only unresolved comments |
| `--format <FMT>` | Output format: `text` (default) or `json` |
| `--all-revisions` | Show comments from all revisions |
| `--include-robot-comments` | Include automated/CI comments |

### grt setup

Set up the current repository for Gerrit (hook, remote, connectivity).

| Flag | Description |
|------|-------------|
| `--remote <NAME>` | Remote name to configure |
| `--force-hook` | Force reinstall of commit-msg hook even if it exists |
| `--remote-hook` | Download hook from remote Gerrit server instead of vendored copy |

### grt export git-review

Create or remove a git-review symlink.

| Flag | Description |
|------|-------------|
| `--clean` | Remove the symlink instead of creating it |

### grt version

Show grt and Gerrit server versions.

### grt completions

Generate shell completions.

**Argument:** `bash` | `zsh` | `fish` | `elvish` | `powershell`

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Generic error (auth failed, not found, server error) |
| 2 | Hook-related error |
| 3 | Malformed input (bad argument format, e.g. invalid CHANGE,PS) |
| 40 | Network/connectivity error |
| 128 | Git config error (no Gerrit host configured) |

## Mutual Exclusivity

The following flag pairs are mutually exclusive and cannot be used together:

- **Topic:** `--topic` / `--no-topic`
- **Rebase:** `--no-rebase` / `--force-rebase` (and `--keep-rebase` conflicts with `--no-rebase`)
- **Track:** `--track` / `--no-track`
- **WIP:** `--wip` / `--ready`
- **Privacy:** `--private` / `--remove-private`
- **Mode flags:** `-d`, `-x`, `-X`, `-N`, `-m`, `-l`, `-s` — only one may be used at a time
