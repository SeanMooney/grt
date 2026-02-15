# Comment JSON Output Schema

Schema for `grt comments --format json` output. Source: `crates/grt/src/comments.rs`.

## Top-Level Structure

```
CommentOutput {
  change: ChangeSummary,
  review_messages: [ReviewMessage],
  inline_comments: [CommentThread],
  summary: CommentSummaryStats
}
```

## Types

### ChangeSummary

```json
{
  "number": 12345,
  "subject": "Fix widget rendering",
  "project": "my-project",
  "branch": "main",
  "status": "NEW",
  "owner": "Alice Smith",
  "owner_email": "alice@example.com",
  "url": "https://review.example.com/c/my-project/+/12345"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `number` | `i64` | Gerrit change number |
| `subject` | `string` | Commit subject line |
| `project` | `string` | Gerrit project name |
| `branch` | `string` | Target branch |
| `status` | `string` | Change status: `NEW`, `MERGED`, `ABANDONED` |
| `owner` | `string` | Change owner display name |
| `owner_email` | `string` | Change owner email |
| `url` | `string` | Gerrit web URL |

### ReviewMessage

```json
{
  "author": "Bob Jones",
  "patch_set": 3,
  "date": "2026-01-15 14:30:00.000000000",
  "message": "Patch Set 3: Code-Review+1\n\nLooks good, minor nit on line 42."
}
```

| Field | Type | Description |
|-------|------|-------------|
| `author` | `string` | Message author display name |
| `patch_set` | `i32?` | Patchset number (null if not associated) |
| `date` | `string` | Timestamp |
| `message` | `string` | Full message text |

### CommentThread

```json
{
  "file": "src/widget.rs",
  "line": 42,
  "resolved": false,
  "comments": [
    {
      "author": "Bob Jones",
      "patch_set": 3,
      "date": "2026-01-15 14:30:00.000000000",
      "message": "This should handle the None case."
    },
    {
      "author": "Alice Smith",
      "patch_set": 3,
      "date": "2026-01-15 15:00:00.000000000",
      "message": "Ack, will fix."
    }
  ]
}
```

| Field | Type | Description |
|-------|------|-------------|
| `file` | `string` | File path relative to repo root |
| `line` | `i32?` | Line number. **`null` means file-level comment.** |
| `resolved` | `bool` | Whether the thread is resolved |
| `comments` | `[ThreadComment]` | Ordered list of comments in the thread |

### ThreadComment

| Field | Type | Description |
|-------|------|-------------|
| `author` | `string` | Comment author display name |
| `patch_set` | `i32?` | Patchset number (null if unknown) |
| `date` | `string` | Timestamp |
| `message` | `string` | Comment text |

### CommentSummaryStats

```json
{
  "total_threads": 8,
  "unresolved": 3,
  "resolved": 5
}
```

| Field | Type | Description |
|-------|------|-------------|
| `total_threads` | `usize` | Total inline comment threads |
| `unresolved` | `usize` | Unresolved threads |
| `resolved` | `usize` | Resolved threads |

## Parsing Instructions

### Finding actionable comments

Filter `inline_comments` where `resolved == false`:

```python
actionable = [t for t in data["inline_comments"] if not t["resolved"]]
```

### Locating code for a comment

Use `file` and `line` to find the exact location:
- `file` is relative to the repo root
- `line` is the 1-based line number in the file
- `line: null` means the comment applies to the entire file, not a specific line

### Getting the latest comment in a thread

The last element of `comments` array is the most recent reply.

### Grouping by file

Comments are already structured per-thread. Group threads by `file` for file-by-file review:

```python
from itertools import groupby
by_file = {f: list(ts) for f, ts in groupby(sorted(actionable, key=lambda t: t["file"]), key=lambda t: t["file"])}
```
