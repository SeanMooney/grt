# SSH Parity Audit

**Status:** Draft  
**Date:** 2025-02

## Summary

Audit of all SSH-related code paths and Gerrit SSH output formats to identify format mismatches that could cause deserialization failures or incorrect behavior.

## SSH Command Paths

| Path | Command | Output Format |
|------|---------|---------------|
| [review_query.rs](crates/grt/src/review_query.rs) | `ssh ... gerrit query --format=JSON project:X status:open` | JSON-per-line (list) |
| [review_query.rs](crates/grt/src/review_query.rs) | `ssh ... gerrit query --format=JSON --current-patch-set --patch-sets change:X` | JSON-per-line (detail) |
| [hook.rs](crates/grt/src/hook.rs) | `scp -O [-P port] userhost:hooks/commit-msg dest` | Binary (hook file) |

## Gerrit SSH JSON Format (from [json.html](https://gerrit-review.googlesource.com/Documentation/json.html))

**Change object:** `project`, `branch`, `topic`, `id`, `number`, `subject`, `owner`, `url`, `createdOn`, `lastUpdated`, `open`, `status`, `currentPatchSet`, `patchSets`, ...

**Account:** `name`, `email`, `username` (no `_account_id` in SSH output)

**patchSet:** `number`, `revision`, `ref`, `uploader`, `author`, `createdOn`, ...

## Findings

### Fixed Previously

1. **AccountInfo.account_id** — SSH omits `_account_id`; made optional.
2. **currentPatchSet missing** — `--patch-sets` alone can omit it; added `--current-patch-set` and fallback to highest patch set.

### Fixed in This Audit

3. **createdOn / lastUpdated** — Added `#[serde(alias = "createdOn")]` and `#[serde(alias = "lastUpdated")]`; flexible deserializer accepts epoch number or string.

4. **id vs change_id** — Use `change_id.or(raw.id)` when building ChangeInfo so SSH output with only `id` works.

5. **patchSet number type** — Custom `deserialize_optional_i32_flexible` accepts integer or string.

6. **Timestamps as epoch numbers** — `deserialize_optional_string_flexible` converts epoch seconds to string for display.

### Unchanged (Same as git-review)

7. **SCP port for SCP-style URLs** — `git@host:project` has no port; we use default 22. Gerrit typically uses 29418. Same behavior as git-review.

### No Issues

- **patchSets structure** — Array of objects with `number`, `ref`, `revision`; matches our `SshPatchSet`.
- **Stats line handling** — Lines with `"type"` are skipped.
- **Hook SCP** — Uses `userhost:hooks/commit-msg`; path is fixed, not from project URL.
- **GIT_SSH** — Respected in `run_gerrit_query_ssh`.
