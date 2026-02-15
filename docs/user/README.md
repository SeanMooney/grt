# grt User Documentation

**grt** is a CLI tool for Git and Gerrit workflows. It provides push, list, download, cherry-pick, compare, and comment retrieval—compatible with git-review as a drop-in replacement.

## What is grt?

grt is a Rust-based command-line tool for working with Gerrit Code Review. It supports:

- Pushing changes to Gerrit for review (with topic, reviewers, WIP/ready flags)
- Listing open changes in a project
- Downloading changes into local branches
- Cherry-picking changes onto your current branch
- Comparing patchsets of a change
- Retrieving review comments (text or JSON)
- Setting up repositories for Gerrit (commit-msg hook, remote, connectivity)

When invoked as `git-review` (via symlink), grt parses the same flat flag syntax as git-review, making it a drop-in replacement for existing workflows.

## Documentation

| Document | Description |
|----------|-------------|
| [Getting Started](getting-started.md) | Quickstart: install, set up a repo, first push, list, and download |
| [Installation](installation.md) | Build from source (cargo, nix), install, git-review export, shell completions |
| [CLI Reference](cli-reference.md) | Complete command and flag reference with exit codes |
| [Configuration](configuration.md) | .gitreview, grt config, credentials, URL rewrites |
| [Workflows](workflows.md) | Push, list, download, cherry-pick, compare, comments, setup |
