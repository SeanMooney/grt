# Getting Started with grt

This guide walks you through installing grt, setting up a Gerrit-hosted repository, and performing your first push, list, and download.

## Prerequisites

- **Rust toolchain** (1.85+) or **Nix** — for building grt
- A **Gerrit-hosted project** — a repository that uses Gerrit for code review (e.g. OpenStack, Android AOSP, or your organization's Gerrit instance)
- **Git** — for version control

## Build and Install

From the grt repository root:

```bash
cargo build --release
```

The binary is produced at `target/release/grt`. To install it on your PATH:

```bash
cargo install --path crates/grt
```

This installs `grt` to `~/.cargo/bin/grt` (ensure `~/.cargo/bin` is in your `PATH`).

See [Installation](installation.md) for Nix, git-review export, and shell completions.

## Set Up a Repository

Clone or open a repository that uses Gerrit. It should have a `.gitreview` file at the root (or you can create one). Example:

```ini
[gerrit]
host=review.example.com
port=29418
project=my/project
defaultbranch=main
defaultremote=gerrit
```

Run setup to install the commit-msg hook, verify the remote, and test connectivity:

```bash
grt setup
```

This will:

1. Install the Gerrit commit-msg hook (adds Change-Id to commits)
2. Verify the configured remote exists (and create it from `.gitreview` if missing)
3. Test connectivity to the Gerrit server

Use `--remote-hook` to download the hook from the Gerrit server instead of the vendored copy.

## First Push

Make a commit (the commit-msg hook will add a Change-Id trailer if configured). Then push to Gerrit:

```bash
grt review
```

Or explicitly specify the target branch:

```bash
grt review main
```

grt will:

1. Check that your commit has a Change-Id
2. Rebase onto the target branch (unless `--no-rebase`)
3. Push to `refs/for/<branch>` with your current branch name as the topic

To add reviewers or mark as work-in-progress:

```bash
grt review --reviewers alice,bob --wip
```

## List Open Changes

To see open changes in the current project:

```bash
grt review -l
```

Brief output shows change number, branch, and subject. For verbose output (adds topic column):

```bash
grt review -ll
```

You can filter by branch:

```bash
grt review -l main
```

## Download a Change

To download change `12345` into a local branch:

```bash
grt review -d 12345
```

grt fetches the change's current patchset, creates a branch (e.g. `review/12345`), and checks it out. You can specify a patchset:

```bash
grt review -d 12345,2
```

## Optional: Use as git-review

To use grt as a drop-in replacement for git-review:

```bash
grt export git-review
```

This creates a symlink at `~/.local/bin/git-review` pointing to grt. When invoked as `git-review`, grt accepts the same flat flags (e.g. `git-review -l`, `git-review -d 12345`).

To remove the symlink:

```bash
grt export git-review --clean
```

## Tips

**SSH keys:** If you use multiple SSH keys or keys with passphrases, consider using `ssh-agent`, Gnome Seahorse, or KDE KWallet to avoid manual SSH configuration or frequent passphrase prompts.

## Next Steps

- [CLI Reference](cli-reference.md) — all commands and flags
- [Configuration](configuration.md) — .gitreview, credentials, URL rewrites
- [Workflows](workflows.md) — cherry-pick, compare, comments, and more
