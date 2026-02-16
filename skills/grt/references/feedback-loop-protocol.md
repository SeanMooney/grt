# Feedback Loop Protocol

Detailed 6-phase protocol for addressing Gerrit review feedback. Referenced from `SKILL.md`.

## Overview

```
Phase 1: Fetch  →  Phase 2: Summarize  →  Phase 3: Plan ← CHECKPOINT
    ↓
Phase 4: Implement  →  Phase 5: Push ← CHECKPOINT  →  Phase 6: Verify
```

Phases 3 and 5 are **mandatory checkpoints** where user approval is required before proceeding.

---

## Phase 1 — Fetch Comments

Run:

```bash
grt comments --format json --unresolved
```

If a specific change is provided:

```bash
grt comments <CHANGE> --format json --unresolved
```

If no change argument is given, grt auto-detects from HEAD's `Change-Id` trailer.

### Error handling

| Exit Code | Action |
|-----------|--------|
| 0 | Continue to Phase 2 |
| 128 | Repo not configured for Gerrit. Run `grt setup` and retry. |
| 40 | Network error. Ask user to check connectivity/VPN. |
| 1 | Auth or not-found error. Verify change number and credentials. |
| 3 | Bad argument format. Check the change number syntax. |

### Empty result

If no unresolved comments exist, report this to the user:

> No unresolved comments found on this change. Nothing to address.

---

## Phase 2 — Summarize

Parse the JSON output (see `references/comment-json-schema.md` for schema).

### Present change metadata

Show:
- Change number, subject, branch, status
- Owner name
- Summary stats: total threads, unresolved count, resolved count

### Present unresolved threads grouped by file

For each file with unresolved comments:

1. **File header** with the file path
2. For each unresolved thread in that file:
   - Line number (or "file-level" if `line` is null)
   - The latest comment message (last in the `comments` array)
   - **Classification** — assign one of:
     - **Bug fix** — reviewer identified a bug or incorrect behavior
     - **Style** — formatting, naming, or code style suggestion
     - **Refactoring** — structural improvement, no behavior change
     - **Question** — reviewer is asking for clarification
     - **Design concern** — reviewer questions the approach or architecture

### Example output format

```
Change 12345: "Fix widget rendering" (main, NEW)
Owner: Alice Smith
Threads: 8 total, 3 unresolved, 5 resolved

## src/widget.rs

Line 42 — [Bug fix]
  Bob: "This should handle the None case — will panic on empty input."

Line 108 — [Style]
  Carol: "Prefer `if let` over `match` here for the single-arm case."

## src/tests/widget_test.rs

File-level — [Question]
  Bob: "Should we add a test for the empty input case?"
```

---

## Phase 3 — Plan (MANDATORY CHECKPOINT)

**Do not proceed without user approval.**

For each unresolved comment:

1. **Read the source file** around the relevant line (use the Read tool with enough context, typically +/- 20 lines).
2. **Propose a response** — one of:
   - **Agree & fix** — describe the specific code change you will make
   - **Partially agree** — explain what you'd change and what you'd keep, and why
   - **Disagree** — explain the technical reasoning (never dismiss without justification)
   - **Needs user input** — you cannot determine the right action alone

3. **Special rules for classifications:**
   - Comments classified as **"Question"** always require user input — present the question and ask the user to answer it
   - Comments classified as **"Design concern"** always require user input — present both sides and ask the user to decide

4. **Present the full plan** to the user in a clear format:

```
## Proposed Changes

### src/widget.rs, line 42 — [Bug fix] Agree & fix
  Reviewer: "This should handle the None case."
  Plan: Add `if value.is_none() { return Ok(default) }` guard before the unwrap on line 42.

### src/widget.rs, line 108 — [Style] Agree & fix
  Reviewer: "Prefer if let over match here."
  Plan: Refactor the match to `if let Some(v) = result { ... }`.

### src/tests/widget_test.rs — [Question] Needs user input
  Reviewer: "Should we add a test for the empty input case?"
  Question for you: Do you want me to add this test? If yes, what should the expected behavior be for empty input?
```

5. **Ask the user to approve, modify, or reject** the plan before continuing.

---

## Phase 4 — Implement

After user approves the plan:

1. **Make the approved changes** using the Edit tool. Only change what was approved.
2. **Run the project test suite** after all changes are made:

```bash
cargo test
```

3. **If tests fail:**
   - Analyze the failure
   - Fix the issue
   - Re-run tests
   - Do not proceed to Phase 5 until tests pass

4. **If tests pass**, proceed to Phase 5.

---

## Phase 5 — Push (MANDATORY CHECKPOINT)

**Do not push without explicit user confirmation.**

### Pre-push checks

1. **Check for uncommitted changes:**

```bash
git status --porcelain
```

2. **Verify Change-Id exists** in the HEAD commit:

```bash
git log -1 --format=%B | grep "Change-Id:"
```

If no Change-Id trailer is found, **stop and alert the user**. Do not amend without a Change-Id.

### Stage and amend

```bash
git add <files that were changed>
git commit --amend --no-edit
```

The `--amend` preserves the existing Change-Id trailer, ensuring the push updates the existing Gerrit change rather than creating a new one.

### Dry-run

```bash
grt review --dry-run
```

Show the dry-run output to the user. It will display the refspec and target branch.

### Wait for confirmation

Present the dry-run output and ask:

> Ready to push this updated patchset to Gerrit. Proceed? (yes/no)

### Push

Only after explicit "yes":

```bash
grt review --yes
```

---

## Phase 6 — Verify (Optional)

After a successful push, optionally verify the new patchset:

```bash
grt comments --format json
```

Confirm:
- The latest patchset number has incremented
- The change is still in the expected state

Report the result to the user:

> Pushed patchset N to change 12345. The change now has N patchsets.

---

## Protocol Rules

1. **Never skip Phase 3 or Phase 5 checkpoints.** Always get user approval before implementing and before pushing.
2. **Batching large reviews.** If there are more than 10 unresolved threads, offer to batch them by file or by classification. Ask the user how they want to proceed.
3. **Always verify Change-Id before amending.** A commit without a Change-Id will create a new change instead of updating the existing one.
4. **Check `git status` for uncommitted changes** before amending. If there are untracked or unstaged changes unrelated to the review feedback, ask the user how to handle them.
5. **Never force-push.** Use `grt review` which pushes to `refs/for/<branch>`, not force-pushing to a branch.
6. **Preserve reviewer intent.** When implementing fixes, match the intent of the reviewer's comment. If uncertain about intent, classify as "needs user input" in Phase 3.
