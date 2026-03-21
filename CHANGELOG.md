# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1] — 2026-03-21

### Added

- Complete git-review drop-in replacement via busybox argv[0] detection
- `grt review`: list, download, cherry-pick, compare, push, rebase modes with full flag parity
- `grt comments`: thread-aware comment retrieval with LLM-optimized text and JSON output
- `grt export git-review`: symlink management for git-review compatibility
- HTTP and SSH Gerrit transport with auto-detection from remote URL
- Shell completions: bash, zsh, fish
- Structured JSON output (`--format json`) for list, download, and push
- Bearer token auth support
- Typed error enum with retry/backoff logic and git-review-compatible exit codes
- SQLite caching foundation (sqlx workspace dep, not yet exposed in CLI)
- Nix flake for reproducible builds (Linux musl + macOS ARM)
