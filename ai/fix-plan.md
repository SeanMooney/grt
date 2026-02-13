# Fix Plan: git-review to grt Port Issues

Plan for fixing all issues documented in `/issues.md`, organized into 6 batches
designed for parallel execution by subagents.

## Batch Overview

| Batch | Focus | Issues | Primary Files | Priority |
|-------|-------|--------|---------------|----------|
| 1 | Config layer fixes | B5,B6,B7,M1,M2,M3,M4,M5,H5(config),L16,L17 | config.rs, app.rs | HIGHEST |
| 2 | Push & refspec fixes | B1,H4,M8,L3 | push.rs, review.rs(L3 only) | HIGH |
| 3 | Main.rs push/flow logic | B2,H2(stub),H3,M6,M7,M9,M14(main),M15,M16,L2,L13,L14,L15,L20 | main.rs | HIGH |
| 4 | Subprocess & Gerrit client | B3,B4,M11,M12,M13,H1,H5(gerrit),L18,L19 | subprocess.rs, gerrit.rs | HIGH |
| 5 | Review/download mode + app | B3(caller),B4(caller),M10,M14(review),L1,L7,L12 | review.rs, app.rs | MEDIUM |
| 6 | List/hook/git cosmetics | L6,L8,L9,L10,L11 | list.rs, hook.rs, git.rs | LOW |

## Execution Order and Parallelism

```
Phase 1 (parallel):
  [Batch 1: config.rs + app.rs]
  [Batch 6: list.rs + hook.rs + git.rs]  (independent, no file conflicts)

Phase 2 (parallel, after Phase 1 completes):
  [Batch 2: push.rs + review.rs(L3)]
  [Batch 4: subprocess.rs + gerrit.rs]

Phase 3 (after Batches 2 + 4 complete):
  [Batch 3: main.rs]  (depends on Batch 2 for H4 ChangeIdStatus)
  [Batch 5: review.rs + app.rs]  (depends on Batch 4 for new subprocess fns)
```

Maximum parallelism: 2 batches per phase. No two parallel batches modify the same file.

---

## Batch 1: Config Layer Fixes

**Issues:** B5, B6, B7, M1, M2, M3, M4, M5, H5 (config portion), L16, L17

**Files:** `config.rs` (primary), `app.rs` (M1 validation)

### Tasks

1. **B5**: Change `GerritConfig::default()` line 51: `scheme` from `"https"` to `"ssh"`.

2. **B6**: Change `GerritConfig::default()` line 49: `branch` from `"main"` to `"master"`.

3. **B7**: In `load_config()` line 252, add check for `gitreview.hostname` in addition to
   `gitreview.host`. Python checks `gitreview.hostname` (`cmd.py:367`).

4. **M1**: In `app.rs`, verify that `config.project` is validated (not just `config.host`).
   Add `anyhow::bail!("no Gerrit project configured")` if project is empty.

5. **M2**: In `parse_gitreview()` line 164, accept `:` as delimiter in addition to `=`:
   ```rust
   trimmed.split_once('=').or_else(|| trimmed.split_once(':'))
   ```

6. **M3**: Lowercase keys on insert in `parse_gitreview()` line 165:
   ```rust
   values.insert(key.trim().to_lowercase(), value.trim().to_string());
   ```

7. **M4**: Add fields to `GerritConfig` for `rebase` (default true), `track` (default false),
   `notopic` (default false), `usepushurl` (default false). Parse from `.gitreview` keys
   `defaultrebase`, `track`, `notopic`, `usepushurl`.

8. **M5**: After loading config, if `scheme == "ssh"` and `ssh_port.is_none()`, set
   `ssh_port = Some(29418)`.

9. **H5** (config only): Read `http.sslVerify` from git config. Check `GIT_SSL_NO_VERIFY`
   env var. Store as `ssl_verify: bool` field on `GerritConfig` (default `true`).

10. **L16**: Stub URL rewriting (`insteadOf`/`pushInsteadOf`) with TODO if too complex.

11. **L17**: Read `gitreview.username` from git config. Store as `username: Option<String>`.

### Tests to update
- `config_defaults()` test: expect `"master"` and `"ssh"`
- Add tests for `:` delimiter, case-insensitive keys, `gitreview.hostname`
- Add test for default SSH port 29418
- Add test for config validation (empty host/project)

---

## Batch 2: Push & Refspec Fixes

**Issues:** B1, H4, M8, L3

**Files:** `push.rs` (primary), `review.rs` (L3 only)

### Tasks

1. **B1**: Remove `no_rebase` -> `submit=false` mapping at `push.rs:75-77`. The `--no-rebase`
   flag controls local rebase, not a Gerrit push option. Remove `no_rebase` from `PushOptions`.

2. **H4**: Refactor `validate_change_id()` to support auto-amend. Add:
   ```rust
   pub enum ChangeIdStatus { Present(String), MissingCanAutoAmend, MissingNeedHook }
   pub fn check_change_id_status(commit_message: &str, hook_installed: bool) -> ChangeIdStatus
   ```
   The caller in `main.rs` (Batch 3) will handle auto-amend logic.

3. **M8**: Add whitespace validation for CC and hashtag values at `push.rs:57-64`, matching
   the reviewer validation at line 51.

4. **L3**: In `review.rs:115-116`, validate `--notify` against allowed values
   `NONE|OWNER|OWNER_REVIEWERS|ALL`. Use a const array or `clap::ValueEnum`.

### Tests to update
- Fix/remove test for `build_refspec_with_no_rebase` (B1)
- Add tests for CC/hashtag whitespace rejection (M8)
- Add test for `--notify` value validation (L3)

---

## Batch 3: Main.rs Push/Flow Logic

**Issues:** B2, H2 (stub), H3, M6, M7, M9, M14 (main.rs), M15, M16, L2, L13, L14, L15, L20

**Files:** `main.rs`

### Tasks

1. **B2**: Guard `--finish` logic at line 479 on `!args.dry_run`.

2. **H2**: Stub rebase workflow. Emit warning when `--force-rebase` is used:
   "pre-push rebase not yet implemented". Wire into push flow with TODO comments.

3. **H3**: Default topic to current branch name when `args.topic` is `None` and
   `args.no_topic` is false (line 460).

4. **M6**: Add `--no-follow-tags` to git push command at line 565.

5. **M7**: Thread `no_thin` from `ReviewArgs`/`PushArgs` to the git push command.
   Build push args dynamically.

6. **M9**: Move `--track` resolution before list dispatch at line 386-394.

7. **M14**: Add `tracing::warn!` for `--color`/`--no-color` when used.

8. **M15**: Enhance multi-commit confirmation. Add "commit exists on remote" warning.
   Wire in auto-amend for single-commit-without-hook (depends on Batch 2 H4).

9. **M16**: When both `--setup` and `--finish` are set, run setup then finish.

10. **L2**: Add `--version` and `--license` flags to `GitReviewCli`.

11. **L13**: Show full command in dry-run output.

12. **L14**: After failed push with "Missing tree", suggest `--no-thin`.

13. **L15**: Add `-c color.remote=always` to push when color enabled.

14. **L20**: Apply `--track` resolution before ALL mode dispatches.

### Dependencies
- Soft dependency on Batch 2 (H4 `ChangeIdStatus`) for M15.

---

## Batch 4: Subprocess & Gerrit Client

**Issues:** B3, B4, M11, M12, M13, H1, H5 (gerrit), L18, L19

**Files:** `subprocess.rs` (primary), `gerrit.rs` (M12, H5)

### Tasks

1. **B3**: Add `git_set_upstream_tracking(branch, upstream, work_dir)` function.

2. **B4**: Add `git_checkout_or_reset_branch(branch, start_point, work_dir)` that handles
   branch-already-exists by falling back to `checkout` + `reset --keep`.

3. **M11**: Set `LANG=C` and `LANGUAGE=C` on all subprocess calls. Refactor `git_output()`
   and `git_exec()` to use a shared command builder.

4. **M12**: Fix `api_url()` in `gerrit.rs:107-114` to append to base URL path instead of
   using `Url::join` with absolute paths (which discards sub-path prefixes).

5. **M13**: Change `git_credential_fill()` to return `Result<Option<(String, String)>>`
   instead of hard-failing on credential helper failure.

6. **H1**: Verify git credential integration already works in `app.rs`. Mark as
   resolved/already-implemented if so.

7. **H5** (gerrit): Accept `ssl_verify: bool` in `GerritClient::new()`. When false,
   configure reqwest with `.danger_accept_invalid_certs(true)`.

8. **L18**: Document credential helper input format difference with code comment.

9. **L19**: Add `check_remote_exists(remote, work_dir)` function.

### Tests to update
- Add test for `git_checkout_or_reset_branch`
- Add test for `LANG=C` env setting
- Add test for sub-path URL construction in `gerrit.rs`
- Add test for credential fill returning None

---

## Batch 5: Review/Download Mode + App

**Issues:** B3 (caller), B4 (caller), M10, M14 (review.rs), L1, L7, L12

**Files:** `review.rs` (primary), `app.rs` (M13 caller update)

### Tasks

1. **B3 caller**: In `cmd_review_download()` line 314, call
   `subprocess::git_set_upstream_tracking()` after checkout.

2. **B4 caller**: Replace `git_checkout_new_branch()` with
   `subprocess::git_checkout_or_reset_branch()`.

3. **M10**: Normalize compare argument through URL parsing before `parse_compare_arg()`.
   Detect URLs starting with `http` and run through `normalize_change_arg()`.

4. **M14 (review.rs)**: Add `tracing::warn!` for unused flags (`--remote-hook`,
   `--use-pushurl`, `--force-rebase`, `--no-custom-script`).

5. **L1**: Add `--keep-rebase` (`-K`) flag to `ReviewArgs` with
   `conflicts_with = "no_rebase"`.

6. **L7**: Document intentional branch naming difference with code comment.

7. **L12**: Document compare mode limitations (no rebase, no validation) with TODO comments.

8. **app.rs**: Update `authenticate()` to handle `Option` return from
   `git_credential_fill()` (M13 caller update).

### Dependencies
- Depends on Batch 4 for new subprocess functions and credential return type.

---

## Batch 6: List/Hook/Git Cosmetics

**Issues:** L6, L8, L9, L10, L11

**Files:** `list.rs`, `hook.rs`, `git.rs`

### Tasks

1. **L6** (list.rs):
   - Add "No pending reviews" message when change list is empty
   - Add "Found N items for review" summary line
   - Use `"-"` instead of empty string for missing fields
   - Use double-space column separator
   - Right-align branch column

2. **L8** (hook.rs): Replace hard-coded `0o755` with adaptive permissions respecting umask.
   Add execute bits matching existing read bits.

3. **L9** (hook.rs): Add submodule hook propagation using
   `git submodule foreach --recursive`.

4. **L10** (hook.rs): Stub `--remote-hook` with informative message. Add TODO for
   HTTP/SCP download.

5. **L11** (git.rs): Add `current_branch_or_default(default: &str)` method that returns
   the default branch name on detached HEAD instead of erroring.

### Dependencies
- None. Fully independent of all other batches.

---

## Global Requirements for All Subagents

1. **SPDX headers**: Every `.rs` file must start with:
   ```rust
   // SPDX-License-Identifier: Apache-2.0 OR MIT
   // Copyright (c) 2026 grt contributors
   ```

2. **Error handling**: Use `anyhow::Result<T>` with `.context("describing the operation")`.

3. **Testing**: Add unit tests in `#[cfg(test)] mod tests` for every changed function.

4. **Validation**: Run `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`,
   and `cargo test` before committing.

5. **Commits**: Use `git commit -s` for DCO sign-off.

6. **Conventions**: Follow `/home/smooney/repos/grt/docs/design/adopted/rust-conventions.md`.

7. **Environment**: Run all cargo commands via `nix develop` shell.
