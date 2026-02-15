# Workflows

This page describes common workflows: push, list, download, cherry-pick, compare, comments, and setup.

## Quick reference

| If you want to… | Then run |
|-----------------|----------|
| Push to the default branch | `grt review` |
| Push to a different branch | `grt review branchname` |
| Push to a different remote | `grt review -r my-remote` |
| Set a review topic | `grt review -t awesome-feature` |
| Add reviewers | `grt review --reviewers alice@example.com bob@example.com` |
| Add CC recipients | `grt review --cc alice@example.com` |
| Push and remove the local branch | `grt review -f` |
| Push without rebasing (e.g. merge conflict) | `grt review -R` |
| Download a change | `grt review -d 781` |
| Download a specific patchset | `grt review -d 781,4` |
| Compare patchsets | `grt review -m 781` or `grt review -m 781,4-10` |
| List open changes | `grt review -l` |
| Set up repository (hook, remote) | `grt review -s` or `grt setup` |

## Push Workflow

Pushing changes to Gerrit involves:

1. **Change-Id check** — Your commit must have a `Change-Id:` trailer. Run `grt setup` to install the commit-msg hook if needed.
2. **Rebase** — By default, grt rebases onto the target branch before pushing. Use `--no-rebase` to skip.
3. **Refspec** — grt pushes to `refs/for/<branch>` with your current branch name as the topic.
4. **Topic** — The topic is derived from your branch name unless you pass `--topic <name>` or `--no-topic`.
5. **Reviewers** — Add reviewers with `--reviewers alice,bob` or `--reviewers alice --reviewers bob`.

Example:

```bash
grt review main --topic my-feature --reviewers alice
```

For work-in-progress or ready-for-review:

```bash
grt review --wip
grt review --ready
```

## Listing Changes

List open changes in the current project:

```bash
grt review -l
```

Brief output shows change number, branch, and subject. For verbose output (adds topic column):

```bash
grt review -ll
```

Filter by branch:

```bash
grt review -l main
```

## Downloading a Change

Download a change into a local branch:

```bash
grt review -d 12345
```

grt fetches the current patchset, creates a branch (e.g. `review/<owner>/<topic>` or `review/<change>/<patchset>`), and checks it out.

To download a specific patchset:

```bash
grt review -d 12345,2
```

You can also pass a Gerrit URL; grt parses change and patchset from it:

```bash
grt review -d https://review.example.com/12345/2
```

## Cherry-picking

Three modes apply a change onto your current branch:

| Flag | Behavior |
|------|----------|
| `-x` / `--cherrypick` | Standard cherry-pick (`git cherry-pick`) |
| `-X` / `--cherrypickindicate` | Cherry-pick with "(cherry picked from ...)" in the commit message |
| `-N` / `--cherrypickonly` | Apply to working directory only (no commit) |

Example:

```bash
grt review -x 12345
grt review -X 12345,2
grt review -N 12345
```

## Comparing Patchsets

Compare mode supports four forms:

| Form | Example | Meaning |
|------|---------|---------|
| Bare change | `grt review -m 12345` | Diff base vs latest patchset |
| Base vs latest (explicit) | `grt review -m 12345,0` | Same as bare change; `0` = base |
| Single patchset | `grt review -m 12345,1` | Diff patchset 1 vs latest (git-review compat) |
| Base vs patchset | `grt review -m 12345,0-3` | Diff base vs patchset 3 (`0` = base sentinel) |
| Patchset range | `grt review -m 12345,1-3` | Diff patchset 1 vs patchset 3 |

Diff order is always `git diff old new` (first = old, second = new). grt fetches the refs and runs `git diff`.

## Retrieving Comments

Fetch review comments for a change:

```bash
grt comments
```

Without arguments, grt uses the Change-Id from the HEAD commit. Specify a change explicitly:

```bash
grt comments 12345
```

Options:

| Flag | Description |
|------|-------------|
| `--revision <REV>` | Show comments for a specific patchset |
| `--unresolved` | Show only unresolved comment threads |
| `--format json` | JSON output for scripting |
| `--all-revisions` | Show comments from all patchsets |
| `--include-robot-comments` | Include automated/CI comments |

## Setup

Configure the repository for Gerrit:

```bash
grt setup
```

This:

1. Installs the commit-msg hook (adds Change-Id to commits)
2. Verifies the configured remote exists (creates it from config if possible)
3. Tests connectivity to the Gerrit server

Flags:

- `--remote <NAME>` — Override the remote name
- `--force-hook` — Reinstall the hook even if it already exists
- `--remote-hook` — Download the hook from the Gerrit server instead of using the vendored copy

Equivalent: `grt review -s`

## Post-push Cleanup

After a successful push, use `--finish` to:

1. Check out the target branch
2. Delete the current topic branch

```bash
grt review --finish
```

This runs automatically after the push completes when `--finish` is set.
