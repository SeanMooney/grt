# git-review Workflow

**Source project:** git-review
**Source files:** `git_review/cmd.py`, `git_review/tests/`
**Status:** Stub
**Informs:** `cli-design.md`, `git-operations.md`

## Overview

<!-- TODO: How git-review manages the push-to-Gerrit workflow -->

## Push Workflow

<!-- TODO: Branch → ref mapping, Change-Id generation, topic setting -->

### Normal Push
### Draft/WIP Push
### Push with Topic
### Push to Specific Branch

## Change-Id Hook

<!-- TODO: commit-msg hook installation, Change-Id format, validation -->

## Rebase Handling

<!-- TODO: How git-review handles rebase before push -->

## Configuration

### Git Config Integration
<!-- TODO: git config keys used (gitreview.remote, gitreview.branch, etc.) -->

### .gitreview File
<!-- TODO: Per-repo config file format and options -->

### Config Precedence
<!-- TODO: .gitreview → git config → defaults -->

## Error Handling

<!-- TODO: How git-review reports errors to users -->

## grt Divergences

<!-- TODO: Where grt's workflow will differ:
- git2 vs subprocess git calls
- Integrated with TUI (not just CLI)
- Potential for batch operations
-->
