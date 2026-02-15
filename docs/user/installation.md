# Installation

This page describes how to build and install grt from source, use Nix, export a git-review symlink, and enable shell completions.

## From Source (Cargo)

### Build

From the grt repository root:

```bash
cargo build --release
```

The binary is produced at `target/release/grt`.

### Install

To install grt on your PATH:

```bash
cargo install --path crates/grt
```

This installs `grt` to `~/.cargo/bin/grt`. Ensure `~/.cargo/bin` is in your `PATH`.

## Nix

### Build

```bash
nix build
```

The built binary is at `result/bin/grt`.

### Install to Profile

```bash
nix profile install .
```

This adds grt to your Nix user profile. The binary is available as `grt` in your PATH.

### Development Shell

To enter a development shell with the Rust toolchain, cargo-deny, cargo-nextest, just, and pre-commit:

```bash
nix develop
```

## Binary Location Summary

| Method | Location |
|--------|----------|
| `cargo install --path crates/grt` | `~/.cargo/bin/grt` |
| `nix build` | `result/bin/grt` |
| `nix profile install .` | Nix profile (in PATH) |

## git-review Compatibility

grt can be used as a drop-in replacement for git-review. When invoked as `git-review` (via symlink), grt parses the same flat flag syntax.

### Create Symlink

```bash
grt export git-review
```

This creates a symlink at `~/.local/bin/git-review` pointing to the grt executable. Ensure `~/.local/bin` is in your `PATH`.

### Remove Symlink

```bash
grt export git-review --clean
```

This removes the `~/.local/bin/git-review` symlink.

## Shell Completions

grt can generate shell completions for bash, zsh, and fish.

### Generate Completions

```bash
grt completions bash   # bash
grt completions zsh    # zsh
grt completions fish   # fish
```

### Enable Completions

**Bash** — add to `~/.bashrc`:

```bash
eval "$(grt completions bash)"
```

Or write to a file and source it:

```bash
grt completions bash > ~/.local/share/bash-completion/completions/grt
# Source from ~/.bashrc if your distro doesn't auto-load that directory
```

**Zsh** — add to `~/.zshrc`:

```bash
eval "$(grt completions zsh)"
```

Or write to a file in your fpath:

```bash
grt completions zsh > ~/.local/share/zsh/site-functions/_grt
```

**Fish** — add to `~/.config/fish/completions/grt.fish`:

```bash
grt completions fish > ~/.config/fish/completions/grt.fish
```
