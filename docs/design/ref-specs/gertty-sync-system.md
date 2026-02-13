# Gertty Sync System

**Source project:** gertty
**Source files:** `gertty/sync.py`, `gertty/db.py` (sync-related methods)
**Status:** Draft
**Informs:** `sync-engine.md`, `gerrit-client.md`

## Overview

Gertty synchronizes a local SQLite database with a remote Gerrit server using a producer/consumer architecture built on Python threads and an in-process priority queue. The system is designed for offline-first operation: the local database is the authoritative data source for the UI, and synchronization happens asynchronously in the background.

The core of the sync engine is the `Sync` class (instantiated once at startup), which owns:

- A `MultiQueue` priority queue that holds pending `Task` objects.
- A `requests.Session` for HTTP communication with the Gerrit REST API.
- A result queue (`queue.Queue`) that delivers `UpdateEvent` objects back to the UI thread.
- A periodic sync thread that re-enqueues subscription syncs every 60 seconds and maintenance tasks every hour.

The main loop (`Sync.run`) pulls tasks from the queue one at a time, executes them synchronously, and writes results to the result queue. A pipe (file descriptor) is used to signal the UI thread that new results are available, triggering a screen refresh.

Tasks are self-contained objects. Each task class implements a `run(sync)` method that receives the `Sync` instance, giving it access to HTTP methods (`sync.get`, `sync.post`, `sync.put`, `sync.delete`) and the ability to enqueue follow-on tasks via `sync.submitTask()`. Tasks can also spawn sub-tasks, tracked via `self.tasks`, and report events via `self.results`.

The startup sequence queues a deterministic set of bootstrap tasks at various priorities:

```python
# HIGH_PRIORITY — must complete before meaningful work begins
self.submitTask(GetVersionTask(HIGH_PRIORITY))
self.submitTask(SyncOwnAccountTask(HIGH_PRIORITY))
self.submitTask(CheckReposTask(HIGH_PRIORITY))
self.submitTask(UploadReviewsTask(HIGH_PRIORITY))
self.submitTask(SyncProjectListTask(HIGH_PRIORITY))

# NORMAL_PRIORITY — core sync work
self.submitTask(SyncSubscribedProjectsTask(NORMAL_PRIORITY))

# LOW_PRIORITY — background maintenance
self.submitTask(SyncSubscribedProjectBranchesTask(LOW_PRIORITY))
self.submitTask(SyncOutdatedChangesTask(LOW_PRIORITY))
self.submitTask(PruneDatabaseTask(self.app.config.expire_age, LOW_PRIORITY))
```

All database access is serialized through a `threading.Lock` on the `Database` object. The `DatabaseSession` context manager acquires this lock on entry and releases it on exit, committing on success and rolling back on exception. This means the sync thread and UI thread never hold the database lock simultaneously, but it also means database access is a global bottleneck.

## Priority Queue

The `MultiQueue` class implements a multi-level priority queue with three levels:

| Constant | Value | Purpose |
|---|---|---|
| `HIGH_PRIORITY` | 0 | Bootstrap tasks, reconnection tasks, pending uploads |
| `NORMAL_PRIORITY` | 1 | Regular sync operations (project sync, change sync) |
| `LOW_PRIORITY` | 2 | Background maintenance (pruning, outdated change re-sync, branch sync) |

### Data structure

Each priority level is backed by a `collections.deque`, stored in an `OrderedDict` keyed by priority value. The `get()` method iterates the deques in priority order (0, 1, 2) and returns the first available item, implementing strict priority scheduling: all HIGH items are drained before any NORMAL item runs, and all NORMAL items before any LOW item.

```python
def get(self):
    self.condition.acquire()
    try:
        while True:
            for queue in self.queues.values():
                try:
                    ret = queue.popleft()
                    self.incomplete.append(ret)
                    return ret
                except IndexError:
                    pass
            self.condition.wait()
    finally:
        self.condition.release()
```

### Deduplication

The `put()` method checks whether the item already exists in the target priority deque using `__eq__` before adding it. Each task class defines `__eq__` based on its identifying fields (e.g., `change_id` for `SyncChangeTask`, `project_keys` for `SyncProjectTask`). If a duplicate is found, the task is not added and `put()` returns `False`, causing `submitTask()` to immediately mark the duplicate as completed (failed).

```python
def put(self, item, priority):
    added = False
    self.condition.acquire()
    try:
        if item not in self.queues[priority]:
            self.queues[priority].append(item)
            added = True
        self.condition.notify()
    finally:
        self.condition.release()
    return added
```

Note that deduplication only checks within the same priority level. A task at NORMAL_PRIORITY is not detected as a duplicate of the same task at HIGH_PRIORITY. Also, deduplication only checks the pending queue, not the currently-executing task (tracked in `self.incomplete`).

### Completion tracking

When a task is dequeued via `get()`, it is moved to the `incomplete` list. After execution, `complete(item)` removes it. The `qsize()` method reports the total of both pending and incomplete items.

### Thread synchronization

The queue uses a `threading.Condition` for producer/consumer coordination. The consumer (`get()`) waits on the condition when all deques are empty. Producers (`put()`) notify the condition after adding an item, waking the consumer.

### Task search

The `find(klass, priority)` method allows searching the queue for tasks of a specific class at a specific priority. This is used by `_syncChangeByCommit()` to batch multiple commit lookups into a single `SyncChangesByCommitsTask`.

## Task Types

Gertty defines 27 distinct task classes, all subclassing `Task`. The base `Task` class provides:

- `priority` — the priority level for queue placement
- `succeeded` — tri-state: `None` (running), `True`, `False`
- `event` — a `threading.Event` for callers to `wait()` on completion
- `tasks` — list of spawned sub-tasks
- `results` — list of `UpdateEvent` objects generated during execution

### Inbound Tasks

These tasks fetch data from the Gerrit server and write it to the local database.

#### Account & Version

| Task | Parameters | Description |
|---|---|---|
| `SyncOwnAccountTask` | none | Fetches `accounts/self` from Gerrit. Stores the authenticated user's account ID, name, username, and email in the local database. Sets `sync.account_id` for later use (e.g., detecting own votes). |
| `GetVersionTask` | none | Fetches `config/server/version`. Parses the version string into a `(major, minor, micro)` tuple stored on `sync.version`. Used for feature gating (e.g., robot comments require >= 2.14.0, commit message editing changed at 2.11.0). Also adjusts `git_url` for Gerrit < 3.0 (adds `/p/` prefix). |

#### Project-Level Sync

| Task | Parameters | Description |
|---|---|---|
| `SyncProjectListTask` | none | Fetches all projects via `projects/?d`. Performs set-difference with local projects: creates new ones, deletes removed ones. Emits `ProjectAddedEvent` for each new project. |
| `SyncSubscribedProjectsTask` | none | Iterates all subscribed projects, batching them into groups of 10 for `SyncProjectTask`. Also enqueues `SyncQueriedChangesTask` for `is:owner` and `is:starred` queries. |
| `SyncProjectTask` | `project_keys: list[int]` | For each project key, builds a Gerrit query scoped to that project. Uses incremental sync: if `project.updated` is set, limits query to changes modified since that timestamp (with a 4-second buffer). Otherwise fetches only open changes. For each returned change, enqueues `SyncChangeTask`. After processing, enqueues `SetProjectUpdatedTask` to record the sync timestamp. |
| `SetProjectUpdatedTask` | `project_key, updated` | Updates `project.updated` in the database. This timestamp is the watermark for incremental sync in `SyncProjectTask`. |
| `SyncProjectBranchesTask` | `project_name` | Fetches `projects/{name}/branches/`, extracts branch names from `refs/heads/*`, and reconciles with the local branch list (creates new, deletes removed). |
| `SyncSubscribedProjectBranchesTask` | none | Iterates subscribed projects and enqueues a `SyncProjectBranchesTask` for each. |

#### Change-Level Sync

| Task | Parameters | Description |
|---|---|---|
| `SyncChangeTask` | `change_id, force_fetch=False` | The most complex task (~400 lines). Fetches a single change with all detail options (`DETAILED_LABELS`, `ALL_REVISIONS`, `ALL_COMMITS`, `MESSAGES`, `DETAILED_ACCOUNTS`, `CURRENT_ACTIONS`, `ALL_FILES`). Sub-fetches comments, robot comments, and checks for each revision. Also queries for conflicting changes. Reconciles all data with local DB: creates/updates change, revisions, files, comments, messages, approvals, labels, permitted labels, conflicts, checks, and hashtags. Performs git fetches for new revisions. Emits `ChangeAddedEvent` or `ChangeUpdatedEvent`. Marks the change `outdated=True` on failure so it can be retried later. |
| `SyncChangeByNumberTask` | `number` | Queries `changes/?q={number}` and enqueues `SyncChangeTask` for each result. Used for on-demand lookup by change number. |
| `SyncChangesByCommitsTask` | `commits: list[str]` | Queries changes by commit SHA (`commit:{sha} OR commit:{sha} ...`). Batches up to 100 commits per query to stay under URL length limits. The `addCommit()` method allows an existing queued task to absorb additional commits. |
| `SyncQueriedChangesTask` | `query_name, query` | Runs an arbitrary Gerrit query (e.g., `is:owner`, `is:starred`) with incremental sync support via `SyncQuery.updated`. Excludes subscribed projects (those are handled by `SyncProjectTask`). Handles pagination via `_more_changes` / `_sortkey` / `start` offset. Enqueues `SyncChangeTask` for each result and `SetSyncQueryUpdatedTask` to record the watermark. |
| `SetSyncQueryUpdatedTask` | `query_name, updated` | Updates the `sync_query.updated` timestamp for incremental queried change sync. |
| `SyncOutdatedChangesTask` | none | Queries the local database for changes marked `outdated=True` and enqueues `SyncChangeTask` for each. These are changes that previously failed to sync. |

#### Repository Integrity

| Task | Parameters | Description |
|---|---|---|
| `CheckReposTask` | none | On startup, iterates all subscribed projects. For any project missing a local git repo, or when `--fetch-missing-refs` is set, enqueues `CheckRevisionsTask`. |
| `CheckRevisionsTask` | `project_key, force_fetch=False` | For a single project, checks all open changes to see if their revision commits and parent commits exist in the local git repo. For any change with missing refs, enqueues `SyncChangeTask` with `force_fetch=True` to re-download the git objects. |

### Outbound Tasks

These tasks upload locally-created data to the Gerrit server. Outbound operations are tracked via `pending_*` boolean columns on the `change` and `revision` tables. The `UploadReviewsTask` serves as the dispatcher that scans for all pending outbound work.

#### Dispatcher

| Task | Parameters | Description |
|---|---|---|
| `UploadReviewsTask` | none | Scans the database for all pending outbound operations using `session.getPending*()` methods and enqueues the appropriate outbound task for each. Queries: `getPendingTopics`, `getPendingHashtags`, `getPendingRebases`, `getPendingStatusChanges`, `getPendingStarred`, `getPendingWIP`, `getPendingCherryPicks`, `getPendingCommitMessages`, `getPendingMessages`. |

#### Individual Outbound Tasks

| Task | Parameters | Description |
|---|---|---|
| `SetTopicTask` | `change_key` | PUTs the change's topic to `changes/{id}/topic`. Clears `pending_topic` flag. Enqueues `SyncChangeTask` to refresh. |
| `SetHashtagsTask` | `change_key` | Fetches current remote hashtags, computes add/remove diff against local hashtags, POSTs to `changes/{id}/hashtags`. Clears `pending_hashtags` flag. Enqueues `SyncChangeTask` to refresh. |
| `RebaseChangeTask` | `change_key` | POSTs to `changes/{id}/rebase`. Clears `pending_rebase` flag. Enqueues `SyncChangeTask` to refresh. |
| `ChangeStatusTask` | `change_key` | POSTs to `changes/{id}/abandon`, `changes/{id}/restore`, or `changes/{id}/submit` depending on `change.status`. Includes optional `pending_status_message`. Clears `pending_status` and `pending_status_message`. Enqueues `SyncChangeTask`. |
| `ChangeStarredTask` | `change_key` | PUTs or DELETEs `accounts/self/starred.changes/{id}` depending on `change.starred`. Clears `pending_starred`. Enqueues `SyncChangeTask`. |
| `ChangeWIPTask` | `change_key` | POSTs to `changes/{id}/wip` or `changes/{id}/ready` depending on `change.wip`. Includes optional `pending_wip_message`. Clears `pending_wip` and `pending_wip_message`. Enqueues `SyncChangeTask`. |
| `SendCherryPickTask` | `cp_key` | POSTs to `changes/{id}/revisions/{commit}/cherrypick` with destination branch and message. Deletes the `PendingCherryPick` record. If the response contains a new change ID, enqueues `SyncChangeTask` for it. |
| `ChangeCommitMessageTask` | `revision_key` | Updates the commit message. Behavior varies by Gerrit version: pre-2.11 uses `POST changes/{id}/revisions/{commit}/message`; 2.11+ uses the edit API (`PUT changes/{id}/edit:message` then `POST changes/{id}/edit:publish`). Checks for existing edits first. Clears `pending_message`. Enqueues `SyncChangeTask`. |
| `UploadReviewTask` | `message_key` | The most complex outbound task. Uploads a review including message, inline comments, and label votes. Before uploading, syncs the change to check for negative votes (hold detection). If the change is held, the upload is skipped. Assembles the review payload: `message`, `labels` (from `draft_approvals`), and `comments` (from `draft_comments` on each file). Deletes the draft records after assembly. If the change is also pending submit, performs the submit in a separate db session (so a submit failure doesn't lose the review). Enqueues `SyncChangeTask` to refresh. |

### Maintenance Tasks

| Task | Parameters | Description |
|---|---|---|
| `PruneDatabaseTask` | `age` | Queries for closed changes older than `age` (e.g., `status:closed age:2months`) and enqueues a `PruneChangeTask` for each. After all prune tasks, enqueues `VacuumDatabaseTask`. |
| `PruneChangeTask` | `key` | Deletes a single change from the local database and removes its git refs from the local repository. Walks all revisions to find fetch refs, deletes them individually, then deletes the parent ref directory. |
| `VacuumDatabaseTask` | none | Executes `VACUUM` on the SQLite database to reclaim disk space after pruning. |

### Task Summary (27 total)

**Inbound (15):** SyncOwnAccountTask, GetVersionTask, SyncProjectListTask, SyncSubscribedProjectsTask, SyncProjectTask, SetProjectUpdatedTask, SyncProjectBranchesTask, SyncSubscribedProjectBranchesTask, SyncChangeTask, SyncChangeByNumberTask, SyncChangesByCommitsTask, SyncQueriedChangesTask, SetSyncQueryUpdatedTask, SyncOutdatedChangesTask, CheckReposTask, CheckRevisionsTask

**Outbound (10):** UploadReviewsTask, SetTopicTask, SetHashtagsTask, RebaseChangeTask, ChangeStatusTask, ChangeStarredTask, ChangeWIPTask, SendCherryPickTask, ChangeCommitMessageTask, UploadReviewTask

**Maintenance (3):** PruneDatabaseTask, PruneChangeTask, VacuumDatabaseTask

## Offline Handling

Gertty's offline handling is built around the principle that the local database is always the user's working copy. The UI never talks to the network directly. All mutations are written to the local database with `pending_*` flags, and the sync engine uploads them when connectivity is available.

### Detection

Offline state is detected by catching connection-related exceptions during task execution:

```python
except (requests.ConnectionError, OfflineError,
        requests.exceptions.ChunkedEncodingError,
        requests.exceptions.ReadTimeout
) as e:
    self.log.warning("Offline due to: %s" % (e,))
    if not self.offline:
        self.submitTask(GetVersionTask(HIGH_PRIORITY))
        self.submitTask(UploadReviewsTask(HIGH_PRIORITY))
    self.offline = True
    self.app.status.update(offline=True, refresh=False)
    time.sleep(30)
    return task
```

Additionally, HTTP 503 responses are treated as offline via the `OfflineError` exception:

```python
def checkResponse(self, response):
    if response.status_code == 503:
        raise OfflineError("Received 503 status code")
```

### Behavior when offline

1. **Failed task is retried.** When a connection error occurs, `_run()` returns the failed task instead of `None`, causing the main loop to re-attempt it after a 30-second sleep.

2. **Queue suppression.** While `self.offline` is `True`, `submitTask()` immediately marks any submitted task as failed (completed with `False`) instead of adding it to the queue. This prevents the queue from filling up with tasks that would all fail.

3. **Reconnection probing.** The retried task serves as the connectivity probe. If it succeeds, `self.offline` is set back to `False` and normal operation resumes.

4. **Recovery bootstrap.** On the first transition to offline, `GetVersionTask` and `UploadReviewsTask` are enqueued at HIGH_PRIORITY. When connectivity returns, these ensure the version is re-validated and any locally-queued outbound operations are uploaded promptly.

5. **UI feedback.** The app's status bar is updated to show the offline indicator, and a pipe write triggers a UI refresh.

### Outbound queue persistence

Outbound operations survive offline periods and even application restarts because they are stored in the database as `pending_*` flags on the `change` and `revision` tables:

- `pending_topic`, `pending_hashtags`, `pending_rebase`, `pending_starred`, `pending_status`, `pending_status_message`, `pending_wip`, `pending_wip_message` on the `change` table
- `pending_message` on the `revision` table
- `pending` on the `message` table (for review uploads)
- `PendingCherryPick` rows in the `pending_cherry_pick` table

On startup (or reconnection), `UploadReviewsTask` scans all these flags and enqueues the appropriate outbound tasks.

## Conflict Resolution

Gertty does not implement general-purpose conflict resolution. Instead, it uses several strategies to minimize conflicts and handle specific cases:

### Server-wins for inbound data

When `SyncChangeTask` fetches a change, the remote data overwrites local state for server-sourced fields (subject, status, topic, updated timestamp, starred, wip). There is no merge: the server version wins unconditionally. This is safe because these fields are either server-controlled or only modified through outbound tasks that clear their pending flags atomically.

### Pending flag guard for outbound data

Outbound operations use a "pending flag" pattern to avoid losing local changes during inbound sync. For example, when the user sets a topic locally:

1. The UI sets `change.topic = "new-topic"` and `change.pending_topic = True`.
2. `SyncChangeTask` fetches the change from the server but does not overwrite `topic` because `pending_topic` acts as a dirty flag (the outbound task reads the local value).
3. `SetTopicTask` uploads the local topic, clears `pending_topic = False`, and enqueues `SyncChangeTask` to refresh.

If the outbound task fails (network error), the pending flag remains set and the database transaction is rolled back (outbound tasks perform HTTP calls inside the `with app.db.getSession()` block, so a failed HTTP call triggers a rollback).

### Draft approval preservation

During inbound change sync, draft approvals (local votes not yet uploaded) are preserved unless a new revision is detected:

```python
for approval in change.approvals:
    if approval.draft and not new_revision:
        # Keep draft approvals — we may be about to upload them.
        user_votes[approval.category] = approval.value
        user_voted = True
        continue
```

If a new revision arrives, draft approvals are discarded because they are no longer valid for the new patchset.

### Held change mechanism

When the sync engine detects that someone left a negative vote after the local user drafted a positive vote, the change is placed in a "held" state:

```python
if user_value > 0 and remote_approval['value'] < 0:
    if not change.held:
        change.held = True
        result.held_changed = True
```

A held change will not have its review uploaded by `UploadReviewTask` — the upload is silently skipped. This prevents the user from inadvertently ignoring negative feedback. The user must explicitly un-hold the change to proceed with submission.

### Reviewed flag management

The `reviewed` flag is cleared (set to `False`) when a new revision or new message arrives from another user, but only if the local user has not voted:

```python
if not user_voted:
    if new_revision or new_message:
        if change.reviewed:
            change.reviewed = False
            result.review_flag_changed = True
```

This prevents the inbound sync from marking a change as unreviewed while the user is actively working on a review.

### Hashtag conflict resolution

`SetHashtagsTask` implements explicit conflict resolution by fetching the current remote hashtags, computing a diff against the local hashtags, and sending an `add`/`remove` delta:

```python
remote_change = sync.get('changes/%s' % change.id)
remote_hashtags = remote_change.get('hashtags', [])
# compute add = local - remote, remove = remote - local
data = dict(add=add, remove=remove)
```

This avoids overwriting hashtags added by other users since the last sync.

### Outdated change recovery

If `SyncChangeTask` fails with a non-connection exception (e.g., unexpected data format, missing data), the change is marked `outdated=True`. `SyncOutdatedChangesTask` periodically re-enqueues these changes for another attempt:

```python
except Exception:
    try:
        with sync.app.db.getSession() as session:
            change = session.getChangeByID(self.change_id)
            if change:
                change.outdated = True
    except Exception:
        self.log.exception("Error while marking change %s as outdated" % (self.change_id,))
    raise
```

## Rate Limiting

Gertty does not implement explicit rate limiting (no token bucket, backoff algorithm, or request-per-second cap). Instead, it relies on several implicit mechanisms:

### Serialized execution

The sync engine processes one task at a time on a single thread. Since each task involves at least one HTTP round-trip plus database I/O, natural serialization limits the request rate. There is no concurrency of HTTP requests.

### Incremental sync with timestamp watermarks

The most impactful rate-reduction mechanism is incremental sync. Both `SyncProjectTask` and `SyncQueriedChangesTask` use timestamp watermarks to limit queries to recently-modified changes:

```python
if project.updated:
    query += ' -age:%ss' % (int(math.ceil((now-project.updated).total_seconds())) + 4,)
else:
    query += ' status:open'
```

The 4-second buffer accounts for request latency and clock skew. After initial sync, subsequent syncs only fetch changes updated since the last watermark, dramatically reducing the volume of data transferred.

### Deduplication

The `MultiQueue.put()` deduplication prevents the same task from being queued multiple times at the same priority, avoiding redundant HTTP requests.

### Commit batching

`_syncChangeByCommit()` batches multiple commit lookups into a single `SyncChangesByCommitsTask`, reducing the number of HTTP requests when many parent commits need to be resolved:

```python
for task in self.queue.find(SyncChangesByCommitsTask, priority):
    if task.addCommit(commit):
        return
# If no existing task can absorb it, create a new one
task = SyncChangesByCommitsTask([commit], priority)
```

Each task batches up to 100 commits per query to stay under URL length limits.

### Pagination

Large result sets from Gerrit are fetched in pages of 500 (the server default) using `_more_changes` and `_sortkey`/`start` offset. This prevents any single request from being too large but does not reduce the total number of requests.

### Fixed HTTP timeout

A 30-second timeout (`TIMEOUT=30`) is applied to all HTTP requests, preventing indefinite hangs. On timeout, the connection error triggers the offline handling path, which includes a 30-second sleep before retry.

### Periodic sync interval

The `periodicSync` thread sleeps for 60 seconds between sync cycles. Hourly maintenance tasks (pruning, outdated change re-sync) run only once per hour. This provides a natural rate floor for background sync.

## grt Divergences

### Tokio async vs Python threading

Gertty uses a single sync thread that blocks on HTTP requests, combined with a periodic sync thread and the main UI thread. The `Database.lock` serializes all database access across threads. This architecture is simple but has inherent limitations:

- Only one HTTP request can be in-flight at a time.
- Database access blocks both the sync thread and UI thread.
- The periodic sync thread can enqueue tasks while the sync thread is blocked on a network request, but cannot make progress itself.

In grt, Tokio's async runtime opens several opportunities:

- **Concurrent HTTP requests.** Multiple `SyncChangeTask` instances could run concurrently using `tokio::spawn` or `JoinSet`, bounded by a semaphore to control concurrency. This would dramatically improve sync throughput for initial sync and large projects.
- **Non-blocking database access.** Using `tokio::task::spawn_blocking` for SQLite operations (rusqlite is synchronous) prevents database queries from blocking the async runtime. Alternatively, a dedicated database actor task with a channel-based interface would serialize writes without blocking callers.
- **Structured concurrency.** Task hierarchies (e.g., `SyncSubscribedProjectsTask` spawning multiple `SyncProjectTask` instances) map naturally to Tokio's `JoinSet` or structured task groups, with automatic cancellation propagation.

### Channel-based task queue vs Python Queue

Gertty's `MultiQueue` is a custom priority queue with condition variable synchronization, deduplication, completion tracking, and task search. In grt, this could be replaced with:

- **`tokio::sync::mpsc`** channels for task submission, with priority sorting handled by a dispatcher task that maintains a `BinaryHeap` or similar structure.
- **A dedicated scheduler task** that receives task submissions, deduplicates, prioritizes, and dispatches to a bounded worker pool.
- **`tokio::sync::oneshot`** channels to replace the `threading.Event` completion notification (each task gets a oneshot sender, caller holds the receiver).

The deduplication logic (checking `__eq__` against all items in the queue) is O(n) in gertty. A `HashSet` or `HashMap` keyed on task identity would provide O(1) deduplication in grt.

The `find()` method used for commit batching could be replaced by a dedicated batching channel or a `DashMap` that accumulates commits before dispatch.

### Potential for incremental/streaming sync

Gertty polls on a fixed interval (60 seconds for project sync, 3600 seconds for maintenance). Several enhancements are possible in grt:

- **Gerrit stream-events.** Gerrit supports an SSH-based event stream (`gerrit stream-events`) that pushes real-time notifications of change updates, comments, and votes. grt could maintain a persistent SSH connection and convert events into targeted sync tasks, eliminating polling latency.
- **Conditional HTTP requests.** Using `If-Modified-Since` or ETags on Gerrit REST calls could reduce bandwidth for unchanged resources.
- **Partial change sync.** Instead of fetching the entire change with all detail options on every sync, grt could fetch only the fields that changed (e.g., just messages, just approvals) based on the event type.
- **Streaming pagination.** Instead of collecting all pages into memory before processing (as gertty does), grt could process each page as it arrives using async streams, reducing peak memory usage for large result sets.
- **Parallel initial sync.** The initial full sync (when `project.updated` is `None`) could fan out across projects concurrently, bounded by a semaphore. Gertty does this sequentially within batches of 10 project keys per `SyncProjectTask`.
