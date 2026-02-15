# grt

A Gerrit review tool built in Rust.

## Overview

grt is a drop-in replacement for git-review. Push changes, list open reviews,
download patchsets, cherry-pick, compare revisions, and retrieve review comments
-- all from the command line.

- Async I/O (tokio + reqwest)
- SSH and HTTP Gerrit transports
- Structured JSON output for programmatic use
- git-review flag compatibility via busybox-style argv[0] detection

## Quick Start

### Install

```bash
# From source
cargo install --path crates/grt

# Or via Nix
nix build
```

### Set up a repository

```bash
cd your-gerrit-project
grt setup
```

This installs the commit-msg hook and verifies Gerrit connectivity.

### Push a change

```bash
grt review              # Push to default branch
grt review main         # Push to specific branch
grt review --dry-run    # Preview without pushing
```

### List open changes

```bash
grt review -l           # Brief listing
grt review -ll          # Verbose (includes topic)
grt review -l --format json   # JSON output
```

### Download a change

```bash
grt review -d 12345       # Latest patchset
grt review -d 12345,2     # Specific patchset
```

### Read review comments

```bash
grt comments                     # Comments for HEAD's change
grt comments 12345               # Comments for a specific change
grt comments --format json       # JSON output for parsing
grt comments --unresolved        # Only unresolved threads
```

## Claude Code Integration

grt ships a Claude Code plugin that teaches the `/grt` skill for Gerrit
workflows. To enable it, register the plugin from the repo root:

```bash
claude plugin add .claude-plugin
```

The skill provides:
- Core workflows: list, download, cherry-pick, compare, push, comments
- A 6-phase feedback loop protocol for addressing review feedback
- Safety rules (dry-run before push, check git status before download)
- Structured JSON parsing for programmatic comment handling

## Documentation

- [Getting Started](docs/user/getting-started.md) -- first-time setup walkthrough
- [CLI Reference](docs/user/cli-reference.md) -- all commands, flags, and exit codes
- [Workflows](docs/user/workflows.md) -- push, list, download, cherry-pick, compare, comments
- [Configuration](docs/user/configuration.md) -- .gitreview, credentials, URL rewrites

## Status

v0.1.0 -- git-review parity complete. 317+ tests passing.

## License

Licensed under Apache-2.0 OR MIT at your option.
