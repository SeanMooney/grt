# Contributing to grt

Thank you for your interest in contributing. This document covers everything you need to know
before submitting code: the license terms, the Developer Certificate of Origin (DCO) requirement,
commit conventions, and the development workflow.

## License

grt is dual-licensed under the [Apache License, Version 2.0](LICENSE-APACHE) and the
[MIT License](LICENSE-MIT), at your option. By contributing to this project you agree that your
contributions will be incorporated under both licenses. The license texts must be preserved in
all forks and distributions; modified source files must carry a notice stating that they have
been changed from the original.

All source files should carry the following SPDX header rather than the full license text:

```rust
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors
```

This applies to new files you create. You do not need to modify the headers of existing files
unless you are making substantive changes to them. A pre-commit hook automatically checks for
and inserts the header on `.rs` files.

## Developer Certificate of Origin (DCO)

This project requires all contributors to sign off on every commit using the
[Developer Certificate of Origin, version 1.1](DCO). This is a lightweight mechanism — no
legal paperwork or external signature process — that creates an auditable record in the git
history confirming that you have the right to submit the code under the project license and that
you are doing so knowingly.

The full DCO text is reproduced in the [DCO](DCO) file at the root of this repository. In
summary, by signing off you certify that:

- the contribution is your own original work, or
- you have the right to submit it under the Apache-2.0 OR MIT license, or
- it has been provided to you by a third party who certified one of the above,

and that you understand the contribution is public and will be maintained indefinitely in the
project's version history.

### How to sign off

Add a `Signed-off-by` trailer to every commit message using the `-s` flag:

```
git commit -s -m "feat(gerrit): add support for NoteDb comment parsing"
```

This produces a commit message of the form:

```
feat(gerrit): add support for NoteDb comment parsing

Signed-off-by: Jane Smith <jane@example.com>
```

The name and email must match your git identity (`git config user.name` and
`git config user.email`). If you use a different address for different contexts, any address
that identifies you is acceptable — the important thing is consistency within a patch series.

To configure git to sign off automatically on all commits in this repository:

```
git config commit.gpgsign false
git config format.signoff true
```

### Amending unsigned commits

If you forget to sign off, amend before submitting:

```
# Single commit
git commit --amend -s

# Multiple commits — interactive rebase
git rebase --signoff HEAD~<n>
```

### DCO enforcement

All pull requests and Zuul patch sets are checked automatically. Commits missing a
`Signed-off-by` trailer will cause the DCO check job to fail. The check validates that every
commit in the series (not just the tip) is signed off.

## Development Setup

### Prerequisites

- Rust toolchain (stable, via `rustup`)
- A Nix installation with flakes enabled, or the dependencies listed in `flake.nix` installed
  manually (see the devShell definition for the authoritative list)
- A reachable Gerrit instance for integration testing (optional for unit/doc tests)

### Building

```
cargo build
```

For a release build:

```
cargo build --release
```

Cross-compilation for supported targets is handled via `cargo-zigbuild` as described in
`build-and-release.md`. You do not need to set this up locally for normal development.

### Running tests

Unit and integration tests (excluding Gerrit integration tests):

```
cargo test
```

To run the full test suite including Gerrit integration tests, a local Gerrit instance is
required. See `tests/integration/README.md` for setup instructions.

Linting and formatting are enforced in CI. Run them locally before pushing:

```
cargo fmt --check
cargo clippy -- -D warnings
```

License and dependency policy checks:

```
cargo deny check
```

### Pre-commit hooks

The project uses [pre-commit](https://pre-commit.com/) to run formatting, linting, SPDX header,
and DCO sign-off checks automatically. Install the hooks once after cloning:

```
just setup-hooks
```

To run all hooks against the entire codebase manually:

```
pre-commit run --all-files
```

## Commit Conventions

This project uses [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).
Commit messages must follow this format:

```
<type>(<scope>): <short description>

[optional body]

[optional footers, including Signed-off-by]
```

### Types

- `feat` — a new feature
- `fix` — a bug fix
- `refactor` — a code change that is neither a feature nor a bug fix
- `test` — adding or correcting tests
- `docs` — documentation changes only
- `chore` — build system, dependency updates, tooling
- `perf` — a change that improves performance
- `ci` — changes to CI configuration (Zuul jobs, GitHub Actions)

### Scopes

Use the primary crate or module affected: `gerrit`, `git`, `db`, `tui`, `cli`, `search`,
`config`, `sync`. Use `workspace` for changes affecting the project as a whole.

### Examples

```
feat(gerrit): implement NoteDb comment thread parsing
fix(db): correct migration ordering for patchset table
refactor(sync): extract retry logic into GerritError::is_retryable
docs(contributing): add DCO sign-off instructions
chore(deps): update tokio to 1.37
```

Breaking changes must include a `BREAKING CHANGE:` footer in the commit body, regardless of
type.

## Pull Requests and Patch Sets

- Keep changes focused. A pull request or Zuul patch set should do one thing. If you find
  yourself fixing an unrelated bug while working on a feature, submit it as a separate change.
- All commits in a series must be individually signed off. The DCO check validates each commit,
  not just the merge commit.
- CI must pass before review. The Zuul check pipeline runs `fmt`, `clippy`, `deny`, and the
  unit test suite. Do not submit a change you know is failing.
- Rebase rather than merge. Keep the history linear. If your branch has fallen behind, rebase
  onto the current tip before submitting.
- Write a meaningful pull request description. Explain what the change does, why it is needed,
  and any non-obvious design decisions. Link to any relevant issues.

## Reporting Issues

Please use the GitHub issue tracker. When reporting a bug, include the output of
`grt --version`, your operating system and Rust toolchain version, and the minimal steps to
reproduce. If the bug involves a Gerrit interaction, include the Gerrit version if known.

Do not include credentials, authentication tokens, or private review content in issue reports.
