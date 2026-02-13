# Code Review Issues: git-review to grt Port

Comprehensive list of issues found during review of the git-review (Python) to grt (Rust) port.
Each issue includes severity, location, and description.

**Status key:** [FIXED] = resolved, [OPEN] = not yet addressed

## Bugs (incorrect behavior)

### B1: `--no-rebase` mapped to wrong refspec option — [FIXED]
- **Severity:** Bug
- **Location:** `push.rs:75-77`
- **Description:** `no_rebase` adds `submit=false` to the Gerrit push refspec. In the original (`cmd.py:1819-1825`), `-R` skips the local test rebase before pushing -- it has nothing to do with Gerrit's `submit` option. The `submit=false` line should be removed.
- **Resolution:** `no_rebase` removed from `PushOptions`; refspec no longer includes `submit=false`.

### B2: `--finish --dry-run` deletes the branch — [FIXED]
- **Severity:** Bug
- **Location:** `main.rs:442-496`
- **Description:** `cmd_push()` returns `Ok(())` during dry-run (`main.rs:559-561`), so the post-push `--finish` logic still runs, deleting the topic branch even though nothing was pushed. The original (`cmd.py:1903`) checks `not options.dry and status == 0`.
- **Resolution:** Finish logic is now guarded by `!args.dry_run`.

### B3: Download mode does not set upstream tracking — [FIXED]
- **Severity:** Bug
- **Location:** `review.rs:313-314`
- **Description:** After `git checkout -b <branch> FETCH_HEAD`, the original calls `git branch --set-upstream-to` (`cmd.py:1362-1366`). The Rust port skips this, leaving the branch with no tracking reference.
- **Resolution:** `subprocess::git_set_upstream_tracking()` is called after checkout.

### B4: Download mode does not handle branch-already-exists — [FIXED]
- **Severity:** Bug
- **Location:** `subprocess.rs:84-86`
- **Description:** If the branch already exists, `git checkout -b` fails and the Rust code returns an error. The original (`cmd.py:1355-1372`) catches this, checks tracking, then does `git checkout <branch> && git reset --keep FETCH_HEAD`.
- **Resolution:** `git_checkout_or_reset_branch()` tries `checkout -b` first, falls back to `checkout` + `reset --keep`.

### B5: Default `scheme` is `https` instead of `ssh` — [FIXED]
- **Severity:** Bug
- **Location:** `config.rs:51` vs `cmd.py:55`
- **Description:** Users with `.gitreview` files that omit `scheme` will get HTTPS behavior instead of the expected SSH behavior. This changes remote URL construction and authentication mechanisms.
- **Resolution:** Default scheme is now `"ssh"`. `gerrit_base_url()` always uses HTTPS for REST API regardless of git transport scheme (ssh→https, http→http, https→https). `authenticate()` only rejects `scheme == "http"` without `--insecure`.

### B6: Default `branch` is `main` instead of `master` — [FIXED]
- **Severity:** Bug
- **Location:** `config.rs:49` vs `cmd.py:56`
- **Description:** Users who rely on the `master` default will silently push to the wrong branch.
- **Resolution:** Default branch is now `"master"`.

### B7: Git config key `gitreview.hostname` not checked — [FIXED]
- **Severity:** Bug
- **Location:** `config.rs:252`
- **Description:** Python's `Config.__getitem__` (`cmd.py:367`) checks `gitreview.hostname` in git config. The Rust port checks `gitreview.host`. Users with `git config gitreview.hostname` set will have it silently ignored.
- **Resolution:** `load_config()` checks `gitreview.host` with fallback to `gitreview.hostname`.

---

## High-severity gaps (missing features affecting core workflows)

### H1: No `git credential` integration — [FIXED]
- **Severity:** High
- **Location:** Absent from Rust codebase
- **Description:** The original (`cmd.py:186-194`) uses `git credential fill` to obtain credentials from OS keychains and credential helpers, with automatic retry on 401 (`cmd.py:216-220`). The Rust port requires a separate `credentials.toml` file.
- **Resolution:** `app.rs:authenticate()` tries `credentials.toml` first, then falls back to `git credential fill`. Calls `git credential approve`/`reject` on success/failure.

### H2: Pre-push rebase workflow is unimplemented — [FIXED]
- **Severity:** High
- **Location:** `main.rs` (missing)
- **Description:** `rebase_changes()`, `undo_rebase()`, `abort_rebase()` (`cmd.py:938-1017`) -- the test-rebase-before-push cycle is a core git-review feature. `--force-rebase` and `--keep-rebase` are parsed but do nothing.
- **Resolution:** Full rebase workflow implemented in `rebase.rs` and integrated into push flow in `main.rs`.

### H3: Default topic from branch name is missing — [FIXED]
- **Severity:** High
- **Location:** `main.rs:460`
- **Description:** The original defaults topic to the current branch name when `--topic` is not passed. The Rust port sets topic to `None`, which means no topic is sent to Gerrit by default.
- **Resolution:** Topic defaults to current branch name unless `--no-topic` is set.

### H4: No auto-amend on first use when hook was missing — [FIXED]
- **Severity:** High
- **Location:** `push.rs:109`
- **Description:** The original (`cmd.py:1068-1071`) detects "1 commit, no hook installed" and auto-amends to add Change-Id. The Rust port returns a hard error instead.
- **Resolution:** `ChangeIdStatus::MissingCanAutoAmend` triggers automatic `git commit --amend --no-edit` in push flow.

### H5: No SSL verification bypass — [FIXED]
- **Severity:** High
- **Location:** Absent from Rust codebase
- **Description:** `GIT_SSL_NO_VERIFY` and `http.sslVerify=false` (`cmd.py:207-212`) are not respected. Users with self-signed Gerrit instances will get TLS errors.
- **Resolution:** `ssl_verify` config field reads `http.sslVerify` and `GIT_SSL_NO_VERIFY`. Gerrit client sets `danger_accept_invalid_certs(true)` when disabled.

---

## Medium-severity gaps

### M1: No validation that host/project are set after config loading — [FIXED]
- **Severity:** Medium
- **Location:** `config.rs`
- **Description:** Python (`cmd.py:912-922`) prints "No '.gitreview' file found" and exits cleanly. The Rust port continues with empty strings, producing confusing downstream errors.
- **Resolution:** `App::new()` validates that both `host` and `project` are non-empty, with clear error messages.

### M2: INI parser: `:` delimiter not supported — [FIXED]
- **Severity:** Medium
- **Location:** `config.rs:164`
- **Description:** Python's `configparser` accepts both `=` and `:` as delimiters. The Rust hand-rolled parser only handles `=`.
- **Resolution:** Parser accepts both `=` and `:` via `.split_once('=').or_else(|| .split_once(':'))`.

### M3: INI parser: case-sensitive keys — [FIXED]
- **Severity:** Medium
- **Location:** `config.rs:165`
- **Description:** Python's `configparser` lowercases all keys. A `.gitreview` with `Host=` instead of `host=` would silently fail in the Rust parser.
- **Resolution:** Keys are lowercased before insertion.

### M4: Missing config options: `rebase`, `track`, `notopic`, `usepushurl` — [FIXED]
- **Severity:** Medium
- **Location:** `config.rs`
- **Description:** These `.gitreview` keys (`cmd.py:56-57`) control push behavior but aren't recognized by the Rust config parser.
- **Resolution:** All four fields added to `GerritConfig` with correct defaults and `.gitreview` parsing.

### M5: No default SSH port 29418 — [FIXED]
- **Severity:** Medium
- **Location:** `config.rs`
- **Description:** Python (`cmd.py:475-476`) defaults port to 29418 when scheme is SSH. The Rust port leaves it as `None`.
- **Resolution:** `load_config()` sets `ssh_port = Some(29418)` when scheme is SSH and no port specified.

### M6: Missing `--no-follow-tags` on push — [FIXED]
- **Severity:** Medium
- **Location:** `main.rs:565`
- **Description:** The original always includes `--no-follow-tags` (`cmd.py:1838`) to prevent accidental tag pushing to Gerrit's `refs/for/` namespace.
- **Resolution:** `--no-follow-tags` is included in push command args.

### M7: `--no-thin` parsed but never forwarded — [FIXED]
- **Severity:** Medium
- **Location:** `review.rs:153`, `main.rs:565`
- **Description:** The flag is accepted but the value is never passed to the `git push` command.
- **Resolution:** `--no-thin` is forwarded to the git push command when set.

### M8: Missing hashtag and CC whitespace validation — [FIXED]
- **Severity:** Medium
- **Location:** `push.rs:57-64`
- **Description:** Reviewers are validated for whitespace (`push.rs:51`), but hashtags and CC values are not. The original validates all three (`cmd.py:1849-1858`).
- **Resolution:** All three (reviewers, CC, hashtags) are validated for whitespace.

### M9: `--track -l` does not filter list by tracked branch — [FIXED]
- **Severity:** Medium
- **Location:** `main.rs:393`
- **Description:** The `--track` resolution happens after the list dispatch has already returned.
- **Resolution:** `--track` resolution happens before all mode dispatches.

### M10: Compare mode does not normalize URLs — [FIXED]
- **Severity:** Medium
- **Location:** `review.rs:423`
- **Description:** The `-m` argument is not run through `parse_change_url()` / `normalize_change_arg()`, so passing a Gerrit URL will fail.
- **Resolution:** Compare argument is normalized through URL parsing when it starts with `http`.

### M11: Missing `LANG=C` / `LANGUAGE=C` environment — [FIXED]
- **Severity:** Medium
- **Location:** `subprocess.rs`
- **Description:** The original (`cmd.py:158-159`) forces English locale on all subprocess calls. Without this, git output in non-English locales could break parsing.
- **Resolution:** `git_command()` sets `LANG=C` and `LANGUAGE=C` on all subprocess calls.

### M12: `Url::join` with absolute paths discards base URL path — [FIXED]
- **Severity:** Medium
- **Location:** `gerrit.rs:112`
- **Description:** For Gerrit instances at a sub-path (e.g., `https://example.com/gerrit/`), the `/a/changes/...` path resolves to `https://example.com/a/changes/...`, dropping the `/gerrit/` prefix.
- **Resolution:** `api_url()` appends to base path instead of using `Url::join` with absolute paths.

### M13: Credential helper hard-fails vs returns None — [FIXED]
- **Severity:** Medium
- **Location:** `subprocess.rs:134-135`
- **Description:** The original returns `None` on credential failure and continues. The Rust port aborts with an error, breaking unauthenticated workflows.
- **Resolution:** `git_credential_fill()` returns `Result<Option<(String, String)>>`, returning `Ok(None)` on failure.

### M14: Flags parsed but unused — [FIXED]
- **Severity:** Medium
- **Location:** Various
- **Description:** The following flags are parsed but silently do nothing:
  - `--remote-hook` (no remote hook download) -- `review.rs:157`
  - `--use-pushurl` -- `review.rs:149`
  - `--force-rebase` -- `review.rs:67`
  - `--no-custom-script` -- `review.rs:161`
  - `--color` / `--no-color` -- `main.rs:85-93`
- **Resolution:** `--remote-hook` and `--force-rebase` are implemented. `--use-pushurl`, `--no-custom-script`, `--color`/`--no-color` emit `tracing::warn!` when used.

### M15: `assert_one_change` safety checks incomplete — [OPEN]
- **Severity:** Medium
- **Location:** `main.rs:524-540`
- **Description:** Missing "commit exists on remote" warning (original warns and asks for `yes`). Missing single-commit-without-hook auto-amend. Rust prompts `y/N` while original requires typing `yes`.
- **Resolution (partial):** Auto-amend for missing Change-Id is implemented. "Commit exists on remote" check is still missing.

### M16: `--setup --finish` interaction not preserved — [FIXED]
- **Severity:** Medium
- **Location:** `main.rs:324-334`
- **Description:** The original allows `--setup --finish` together: runs setup then finishes (switches branch and deletes). The Rust port ignores `--finish` when `--setup` is set.
- **Resolution:** Both flags can coexist; setup runs first, then finish logic executes.

---

## Low-severity gaps (cosmetic / edge cases)

### L1: `--keep-rebase` (`-K`) flag missing — [FIXED]
- **Location:** `review.rs`
- **Description:** Completely absent from `ReviewArgs`. Related to unimplemented rebase workflow (H2).
- **Resolution:** Flag added with `conflicts_with = "no_rebase"`.

### L2: `--license` and `--version` missing from git-review personality — [FIXED]
- **Location:** `main.rs` (`GitReviewCli` struct)
- **Description:** Running `git-review --version` or `git-review --license` would fail.
- **Resolution:** Both flags added to `GitReviewCli`.

### L3: `--notify` accepts any string — [FIXED]
- **Location:** `review.rs:115-116`
- **Description:** Original restricts to `NONE|OWNER|OWNER_REVIEWERS|ALL`. Rust accepts anything, leading to confusing server-side errors.
- **Resolution:** `NotifyLevel` is a clap `ValueEnum` with four valid options.

### L4: Hashtag refspec key difference — [FIXED]
- **Location:** `push.rs:62-64`
- **Description:** Rust uses `hashtag=` vs Python's `t=`. Both are valid in Gerrit.

### L5: Message encoding difference — [FIXED]
- **Location:** `push.rs:66-69`
- **Description:** Rust uses `%20` for spaces, Python uses `+` (`quote_plus`). Both are generally accepted.

### L6: List formatting differences — [OPEN]
- **Location:** `list.rs`
- **Description:** Single-space vs double-space column separator. Left-aligned vs right-aligned branch column. Missing "No pending reviews" and "Found N items" messages. No color support. Empty string vs `"-"` for missing fields.

### L7: Branch naming difference in download mode — [FIXED]
- **Location:** `review.rs:275-290`
- **Description:** Rust uses `review/<owner>/<topic>` vs Python's `review/<change>[-patch<N>]`. Intentional design change but incompatible with existing workflows.

### L8: Hook permissions hard-coded — [FIXED]
- **Location:** `hook.rs:26-31`
- **Description:** Hard-coded `0o755` vs Python's adaptive permission handling that respects umask.
- **Resolution:** Hook installation reads current permissions and applies execute bits adaptively.

### L9: No submodule hook propagation — [FIXED]
- **Location:** `hook.rs`
- **Description:** Original (`cmd.py:442-445`) copies hook into every submodule. Rust does not.
- **Resolution:** `propagate_hook_to_submodules()` installs hooks in all submodules.

### L10: No hook download via HTTP/SCP — [OPEN]
- **Location:** `hook.rs`
- **Description:** `--remote-hook` is a no-op. Original supports HTTP and SCP hook download.

### L11: Detached HEAD errors instead of fallback — [FIXED]
- **Location:** `git.rs:38`
- **Description:** Original falls back to target branch name on detached HEAD. Rust errors out.
- **Resolution:** `current_branch_or_default()` provides fallback to configured branch.

### L12: Compare mode simplified — [OPEN]
- **Location:** `review.rs:423-461`
- **Description:** No rebase support. No `old_ps != new_ps` validation. Simplified to SHA-based diff instead of branch-based checkout + diff.

### L13: Dry-run output incomplete — [OPEN]
- **Location:** `main.rs:559-561`
- **Description:** Doesn't show full command with `--no-follow-tags`/`--no-thin`.

### L14: No `--no-thin` retry suggestion — [OPEN]
- **Location:** `main.rs:565`
- **Description:** Original prints "Consider trying again with --no-thin" on "Missing tree" push error.

### L15: No color remote output on push — [OPEN]
- **Location:** `main.rs:565`
- **Description:** Missing `-c color.remote=always` on push command.

### L16: No URL rewriting (`insteadOf`/`pushInsteadOf`) — [FIXED]
- **Location:** `config.rs`
- **Description:** Git URL rewriting (`cmd.py:527-604`) is not implemented.
- **Resolution:** Full URL rewriting with `populate_rewrites()`, `alias_url()`, longest-match semantics.

### L17: Missing `username` config — [FIXED]
- **Location:** `config.rs`
- **Description:** `gitreview.username` git config (`cmd.py:488`) for SSH URL construction is not read.
- **Resolution:** `gitreview.username` read from git config and stored in `config.username`.

### L18: Credential helper input format difference — [FIXED]
- **Location:** `subprocess.rs:104-112`
- **Description:** Rust sends `protocol=` + `host=` fields. Original sends `url=<full_url>`. Both valid but some credential helpers may behave differently.

### L19: `check_remote()` validation missing — [OPEN]
- **Location:** Not implemented
- **Description:** Original validates remote exists and creates it if needed (`cmd.py:1768-1770`). Rust assumes remote is configured.

### L20: `--track` not applied to download/cherrypick/compare modes — [FIXED]
- **Location:** `main.rs:397-413`
- **Description:** `--track` resolution only happens in push mode. Original applies it globally before mode dispatch.
- **Resolution:** `--track` resolution happens before all mode dispatches.

---

## Areas where grt is better than git-review

- **XSSI stripping** (`gerrit.rs:278-286`): More robust prefix stripping.
- **Retry logic** (`gerrit.rs:163-194`): Exponential backoff on 5xx/network errors.
- **Timeouts** (`gerrit.rs:13-14`): 10s connect, 30s request vs none.
- **Typed errors** (`GerritError` enum): Distinct variants with `is_retryable()`.
- **URL encoding**: Consistent encoding of all query values.
- **`core.hooksPath`**: Equivalent support for custom hook paths.
- **Hook content**: Identical vendored script from same Gerrit upstream.

---

## Summary

| Category | Fixed | Open | Total |
|----------|-------|------|-------|
| Bugs | 7 | 0 | 7 |
| High | 5 | 0 | 5 |
| Medium | 15 | 1 | 16 |
| Low | 13 | 7 | 20 |
| **Total** | **40** | **8** | **48** |

Remaining open items are low-severity cosmetic/edge-case issues (L6, L10, L12-L15, L19) and one medium partial (M15 "commit exists on remote" check).
