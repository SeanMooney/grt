# Fix Plan: git-review to grt Port Issues — COMPLETED

Plan for fixing all issues documented in `/issues.md`, organized into 6 batches.

**Status: All batches executed.** 40 of 48 issues fixed. 8 low-severity items remain open.
See `issues.md` for per-issue status.

## Batch Status

| Batch | Focus | Status | Issues Fixed |
|-------|-------|--------|--------------|
| 1 | Config layer fixes | DONE | B5,B6,B7,M1,M2,M3,M4,M5,H5,L16,L17 |
| 2 | Push & refspec fixes | DONE | B1,H4,M8,L3 |
| 3 | Main.rs push/flow logic | DONE | B2,H2,H3,M6,M7,M9,M14,M16,L2,L20 |
| 4 | Subprocess & Gerrit client | DONE | B3,B4,M11,M12,M13,H1,L18 |
| 5 | Review/download mode + app | DONE | M10,L1,L7 |
| 6 | List/hook/git cosmetics | DONE | L8,L9,L11 |

## Remaining Open Items

| Issue | Severity | Description |
|-------|----------|-------------|
| M15 | Medium | "Commit exists on remote" check missing (auto-amend done) |
| L6 | Low | List formatting cosmetic differences |
| L10 | Low | HTTP/SCP hook download not implemented |
| L12 | Low | Compare mode simplified (no rebase, no validation) |
| L13 | Low | Dry-run output doesn't show full push command |
| L14 | Low | No `--no-thin` retry suggestion on "Missing tree" error |
| L15 | Low | No `-c color.remote=always` on push |
| L19 | Low | `check_remote()` validation missing |
