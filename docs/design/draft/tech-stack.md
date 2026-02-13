# Tech Stack Design Document: Git/Gerrit Workflow Tool

**Version:** 0.1  
**Date:** February 11, 2026  
**Status:** Initial Design

## Executive Summary

This document outlines the technical stack and architecture for a command-line and TUI-based tool for managing Git and Gerrit workflows with local SQLite caching. The tool provides both interactive (TUI) and non-interactive (CLI) modes, with asynchronous operations, fuzzy search capabilities, and direct access to Gerrit's NoteDb format.

## Core Design Principles

1. **Dual Interface**: Support both TUI (interactive) and CLI (scriptable) modes
2. **Async-First**: All I/O operations are asynchronous to maintain responsiveness
3. **Local-First**: Cache data locally in SQLite for offline access and fast queries
4. **Structured Concurrency**: Use scoped tasks for UI-coordinated work, allow long-running background tasks when needed
5. **Observable**: Comprehensive logging and tracing for debugging and production monitoring

## Technology Stack Overview

### Language & Runtime

#### Rust (Edition 2021)
**Role:** Primary programming language  
**Selection Rationale:**
- Memory safety without garbage collection
- Strong async/await support via Tokio
- Excellent ecosystem for CLI tools (clap, ratatui)
- Zero-cost abstractions for performance-critical paths
- Cross-platform support

#### Tokio (v1.x)
**Role:** Async runtime and task scheduler  
**Selection Rationale:**
- De facto standard for async Rust
- Robust multi-threaded work-stealing scheduler
- Rich ecosystem of compatible crates
- Built-in utilities (channels, timers, sync primitives)

**Key Features Used:**
- `tokio::spawn()` - For long-lived background tasks with 'static lifetime
- `tokio::select!` - Event multiplexing in main event loop
- `tokio::time::interval()` - Periodic tick generation
- `mpsc` channels - Event communication between tasks

#### tokio-scoped (v0.2)
**Role:** Structured concurrency for UI-coordinated tasks  
**Selection Rationale:**
- Enables borrowing in spawned tasks (no 'static requirement)
- Guarantees all spawned tasks complete before scope exits
- Clean shutdown semantics for TUI components
- Eliminates need for Arc<Mutex<>> wrapper patterns

**Architecture Pattern:**
```
tokio_scoped::scope(|scope| {
    scope.spawn(/* keyboard input - borrows tx */);
    scope.spawn(/* periodic ticks - borrows tx */);
    scope.spawn(/* main event loop - borrows app state */);
});
// All tasks guaranteed complete here
```

**When to Use:**
- Input handlers that should terminate with TUI
- Short-lived background tasks coordinated with UI
- Any task that needs to borrow app state

**When NOT to Use:**
- Long-running background tasks that outlive TUI session
- Tasks that need to continue after scope exits
- Fire-and-forget operations

### User Interface Layer

#### ratatui (v0.28)
**Role:** Terminal User Interface framework  
**Selection Rationale:**
- Pure Rust TUI library (no ncurses dependency)
- Immediate mode rendering model (React-like)
- Composable widget system
- Excellent performance with minimal flicker

**Key Features:**
- `ratatui::init()` / `restore()` - Automatic terminal setup/cleanup with panic hooks
- Widget system - Block, List, Paragraph, etc.
- Layout constraints - Flexible responsive layouts

**Integration Pattern:**
- Separate rendering from state management
- Draw function takes immutable reference to state
- Event-driven re-renders (not continuous polling)

#### crossterm (v0.28)
**Role:** Cross-platform terminal manipulation  
**Selection Rationale:**
- Works on Windows, macOS, Linux
- Async event stream (no polling required)
- Raw mode, alternate screen, cursor control

**Usage Pattern:**
- `EventStream` - Async stream of keyboard/mouse events
- Integrated with tokio::select! for event multiplexing

#### clap (v4.x)
**Role:** Command-line argument parsing  
**Selection Rationale:**
- Derive-based API (declarative)
- Automatic help generation
- Subcommand support
- Type-safe argument parsing

**Command Structure:**
```
git-gerrit-tool [OPTIONS] [COMMAND]
  --database <PATH>
  --repo <PATH>
  --gerrit-server <URL>
  --log-level <LEVEL>
  
Commands:
  tui           # Launch TUI (default)
  list          # List changes (CLI)
  sync          # Sync from Gerrit (CLI)
  search        # Fuzzy search (CLI)
  show          # Show change details (CLI)
  cherry-pick   # Cherry-pick change (CLI)
  init          # Initialize database (CLI)
```

### Data Layer

#### SQLite via sqlx (v0.8)
**Role:** Local data cache and persistence  
**Selection Rationale:**
- Embedded database (no separate process)
- Excellent SQL support with compile-time query checking
- Async API compatible with Tokio
- Zero-configuration required

**Schema Design:**
```sql
CREATE TABLE changes (
    id TEXT PRIMARY KEY,           -- Gerrit change ID
    subject TEXT NOT NULL,
    status TEXT NOT NULL,          -- NEW, MERGED, ABANDONED
    owner TEXT NOT NULL,
    branch TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    created INTEGER NOT NULL,      -- Unix timestamp
    updated INTEGER NOT NULL       -- Unix timestamp
);

CREATE INDEX idx_changes_status ON changes(status);
CREATE INDEX idx_changes_updated ON changes(updated);
```

**Future Schema Extensions:**
- `patchsets` table - Multiple revisions per change
- `comments` table - Inline comments
- `votes` table - Code-Review, Verified, etc.
- `labels` table - Custom label configuration

**Data Flow:**
1. Fetch from Gerrit REST API → Parse JSON
2. Transform to internal Change model
3. Upsert to SQLite (INSERT ... ON CONFLICT DO UPDATE)
4. Query from SQLite for display/search

#### git2 (v0.19)
**Role:** Git repository operations and NoteDb access  
**Selection Rationale:**
- Rust bindings to libgit2
- Direct repository manipulation without shelling out
- Access to Git internals (refs, notes, objects)

**Primary Use Cases:**
1. **Cherry-pick operations**: Apply commits from Gerrit changes
2. **NoteDb reading**: Direct access to Gerrit's metadata stored in Git
3. **Repository status**: Current branch, dirty state, etc.

**NoteDb Integration:**
Gerrit stores review metadata in special Git refs:
- `refs/changes/XX/NNNN/meta` - Change metadata (status, owner, etc.)
- `refs/changes/XX/NNNN/N` - Patchset N commits
- `refs/notes/review` - Review comments as Git notes
- `refs/meta/config` - Project configuration (labels, submit rules)

**Gap:** Full NoteDb parsing logic needs detailed implementation. Current design provides structure but requires reverse-engineering Gerrit's exact format.

### External Integration

#### reqwest (v0.12)
**Role:** HTTP client for Gerrit REST API  
**Selection Rationale:**
- Async-first design
- JSON serialization/deserialization
- Connection pooling
- Redirect handling

**Gerrit API Interactions:**
```
GET /changes/?q=<query>&o=CURRENT_REVISION&o=CURRENT_COMMIT
  → Fetch changes with current patchset info

GET /changes/{change-id}/detail
  → Get full change details

POST /changes/{change-id}/revisions/{revision-id}/review
  → Submit review (future feature)
```

**Authentication:** Currently not implemented. Future options:
- HTTP Basic Auth
- Digest Auth
- OAuth 2.0
- Cookie-based auth

**Gap:** Authentication mechanism needs design decision based on target Gerrit instance configuration.

### Search & Filtering

#### nucleo-matcher (v0.3)
**Role:** Fuzzy finding algorithm  
**Selection Rationale:**
- Same algorithm used by fzf
- Excellent performance
- Scoring system for result ranking
- Case-insensitive matching

**Search Strategy:**
1. Load candidates from SQLite (up to N results)
2. Build searchable text: `"{subject} {id} {owner}"`
3. Score each candidate against query
4. Sort by score (descending)
5. Return top matches

**Gap:** Advanced search syntax (e.g., field:value filters) not implemented. Consider adding:
- `status:NEW` - Filter by status
- `owner:alice` - Filter by owner
- `branch:main` - Filter by branch
- Combining filters with fuzzy match on remaining fields

### Observability

#### tracing (v0.1) + tracing-subscriber (v0.3)
**Role:** Structured logging and distributed tracing  
**Selection Rationale:**
- Structured logs (key-value pairs)
- Span-based tracing (track execution context)
- Multiple output formats (human-readable, JSON)
- Low overhead

**Configuration:**
- Log levels: TRACE, DEBUG, INFO, WARN, ERROR
- Environment variable: `RUST_LOG=git_gerrit_tool=debug`
- CLI flags: `--log-level`, `--log-file`, `--log-json`

**Key Macros:**
- `#[instrument]` - Auto-create span for function
- `info!()`, `debug!()`, `warn!()`, `error!()` - Log events
- Structured fields: `info!(count = 42, "Operation complete")`

**Output Formats:**
1. **Human-readable** (development): Colorized, timestamp, level, message
2. **JSON** (production): Structured for log aggregation (e.g., ELK stack)

#### tracing-appender (v0.2)
**Role:** Log file rotation  
**Selection Rationale:**
- Daily log rotation
- Non-blocking writes (dedicated thread)
- Automatic cleanup of old logs

**Gap:** Log retention policy and compression not implemented. Consider:
- Keep last N days
- Compress rotated logs (gzip)
- Maximum disk space limits

### Testing Infrastructure

#### Core Testing Crates

**tokio-test (v0.4)**
- Testing async code
- Mock time for testing intervals/timeouts

**tempfile (v3)**
- Temporary directories for test isolation
- Automatic cleanup

**mockito (v1.5)**
- HTTP mock server
- Test Gerrit API integration without real server

**serial_test (v3)**
- Serialize tests that share resources (SQLite database)
- Prevent test flakiness from concurrent access

**proptest (v1)**
- Property-based testing
- Generative testing for edge cases
- Example: Ensure fuzzy search never panics on arbitrary input

**test-case (v3)**
- Parameterized tests
- Reduce boilerplate for similar test cases

#### Test Organization

```
tests/
├── common/
│   └── mod.rs              # Shared test utilities (TestContext, fixtures)
├── db_tests.rs             # Database operations
├── fuzzy_tests.rs          # Search algorithm
├── app_integration_tests.rs # End-to-end workflows
├── gerrit_mock_tests.rs    # Gerrit API with mocks
└── property_tests.rs       # Property-based tests
```

**Test Categories:**
1. **Unit tests** - Single function/module (`src/` files with `#[cfg(test)]`)
2. **Integration tests** - Component interaction (`tests/*.rs`)
3. **Property tests** - Generative testing with arbitrary inputs
4. **Mock tests** - External API interactions

**Gap:** No performance benchmarks. Consider adding:
- SQLite query performance under load
- Fuzzy search performance with large datasets (10k+ changes)
- TUI rendering performance

## Application Architecture

### Module Structure

See [repo-layout.md](../adopted/repo-layout.md) for the module layout.

### Component Interaction Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                         main.rs                              │
│  - Parse CLI args (clap)                                     │
│  - Setup logging (tracing)                                   │
│  - Route to TUI or CLI command                               │
└────────────┬────────────────────────────────────────────────┘
             │
             ▼
    ┌────────────────────┐
    │      App           │  ◄──────── Central orchestrator
    │  - Database        │
    │  - GitRepo         │
    │  - GerritClient    │
    │  - NoteDbReader    │
    └────────┬───────────┘
             │
    ┌────────┴────────┬─────────────┬──────────────┐
    │                 │             │              │
    ▼                 ▼             ▼              ▼
┌─────────┐    ┌──────────┐  ┌──────────┐  ┌────────────┐
│Database │    │GitRepo   │  │Gerrit    │  │NoteDbReader│
│(SQLite) │    │(git2)    │  │Client    │  │(git2)      │
│         │    │          │  │(reqwest) │  │            │
└─────────┘    └──────────┘  └──────────┘  └────────────┘
    │                 │             │              │
    │                 │             │              │
    ▼                 ▼             ▼              ▼
[Local DB]     [Git Repo]   [Gerrit HTTP]  [NoteDb Refs]
```

### Data Flow: TUI Mode

```
┌──────────────────────────────────────────────────────────────┐
│                    TUI Event Loop                             │
│                                                                │
│  ┌─────────────┐  ┌─────────────┐  ┌──────────────┐         │
│  │ Input Task  │  │ Tick Task   │  │Background    │         │
│  │(EventStream)│  │(interval)   │  │Tasks         │         │
│  └──────┬──────┘  └──────┬──────┘  └──────┬───────┘         │
│         │                 │                 │                 │
│         └────────┬────────┴────────┬────────┘                │
│                  │                 │                          │
│                  ▼                 ▼                          │
│          ┌──────────────────────────────┐                    │
│          │  mpsc::channel (TuiEvent)    │                    │
│          └───────────┬──────────────────┘                    │
│                      │                                        │
│                      ▼                                        │
│          ┌──────────────────────┐                            │
│          │  tokio::select!      │                            │
│          │  - Render frame      │                            │
│          │  - Wait for event    │                            │
│          │  - Handle event      │                            │
│          │  - Update state      │                            │
│          └──────────────────────┘                            │
│                                                                │
└──────────────────────────────────────────────────────────────┘
```

**Key Flow:**
1. Three independent event sources feed into channel
2. Main loop uses `tokio::select!` to wait for any event
3. On event arrival: update state, then render
4. Rendering is fast (immediate mode) - no blocking
5. Loop continues until quit event

### Data Flow: CLI Mode

```
User Command
     │
     ▼
┌─────────────────┐
│ clap Parser     │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ App Method      │  (e.g., list_changes, sync_from_gerrit)
└────────┬────────┘
         │
    ┌────┴────┬──────────┐
    ▼         ▼          ▼
[Database] [Gerrit]  [NoteDb]
    │         │          │
    └────┬────┴────┬─────┘
         │         │
         ▼         ▼
    [Results] [Changes]
         │         │
         └────┬────┘
              │
              ▼
    ┌──────────────────┐
    │ Format & Print   │
    └──────────────────┘
```

**Key Flow:**
1. Synchronous-looking API (internally async)
2. Direct result printing (no TUI overhead)
3. Single operation, then exit
4. Ideal for scripting and automation

### Concurrency Model

#### Scoped Tasks (tokio-scoped)
**Lifetime:** Bounded to TUI session  
**Characteristics:**
- Can borrow from parent scope
- Guaranteed to complete before scope exits
- No 'static requirement

**Use Cases:**
- Keyboard input handler
- Periodic tick generator
- Main event loop
- Short-lived background tasks

**Example:**
```rust
tokio_scoped::scope(|scope| {
    scope.spawn(async {
        // Can borrow tx without 'static
        let mut event_stream = EventStream::new();
        while let Some(event) = event_stream.next().await {
            tx.send(TuiEvent::Input(event)).unwrap();
        }
    });
});
// All tasks complete here
```

#### Unscoped Tasks (tokio::spawn)
**Lifetime:** 'static (until completion or program exit)  
**Characteristics:**
- Cannot borrow from parent
- Requires owned data or Arc<T>
- Continues after scope exits

**Use Cases:**
- Long-running background sync
- Periodic data refresh (every 5 minutes)
- Ad-hoc operations triggered by user

**Example:**
```rust
let tx_clone = tx.clone();  // Need owned clone
tokio::spawn(async move {
    loop {
        tokio::time::sleep(Duration::from_secs(300)).await;
        let changes = fetch_from_gerrit().await;
        tx_clone.send(TuiEvent::SyncComplete(changes)).unwrap();
    }
});
```

#### Hybrid Pattern
**Strategy:** Use both patterns together
- Scoped tasks for UI coordination
- Unscoped tasks for independent work
- Communicate via channels

**Advantage:**
- Best of both worlds
- No unnecessary 'static constraints
- Clear lifetime semantics

## Configuration Management

**Gap:** Configuration file support not implemented.

**Proposed:**
```toml
# ~/.config/git-gerrit-tool/config.toml

[gerrit]
server = "https://gerrit.example.com"
username = "alice"
# password in keyring or prompt

[database]
path = "~/.local/share/git-gerrit-tool/gerrit.db"

[git]
repo = "~/projects/myproject"

[ui]
theme = "dark"  # Future: color schemes
refresh_interval = 30  # seconds

[logging]
level = "info"
file = "~/.local/share/git-gerrit-tool/app.log"
json = false
```

**Implementation Options:**
- `toml` crate for parsing
- `config` crate for layered configuration (file → env → CLI)
- `directories` crate for platform-specific paths

## Security Considerations

### Credential Management
**Current State:** Not implemented  
**Required:**
- Gerrit authentication (HTTP Basic, OAuth, etc.)
- Secure storage (OS keyring integration)

**Proposed:**
- `keyring` crate - Cross-platform credential storage
- Support for `.netrc` file (legacy)
- Prompt for password if not stored

### Input Validation
**Current State:** Basic validation via clap  
**Required:**
- Sanitize SQL inputs (sqlx handles this via parameterization)
- Validate Git ref names before operations
- Validate URLs before HTTP requests

### Audit Logging
**Gap:** No audit trail for destructive operations

**Proposed:**
- Log all cherry-picks, merges, abandons
- Include user, timestamp, change ID
- Separate audit log file with retention policy

## Performance Considerations

### Database Optimization
**Current:**
- Indexes on status and updated columns
- Limit clause to prevent unbounded queries

**Future Optimizations:**
- Full-text search index for subject/owner
- Materialized views for common queries
- VACUUM on schedule to reclaim space

### Memory Management
**Current:**
- Load all changes into memory for fuzzy search
- Limit of 1000 changes hardcoded

**Future Optimizations:**
- Streaming results for large datasets
- Cursor-based pagination
- Background indexing for instant search

### Network Efficiency
**Current:**
- Simple HTTP GET requests
- No caching headers

**Future Optimizations:**
- ETag support for conditional requests
- Compression (gzip)
- Batch requests where possible
- Connection pooling (reqwest does this)

## Error Handling Strategy

### Error Types
**Current:** Using `anyhow::Result<T>` everywhere

**Advantages:**
- Simple, ergonomic
- Context attachment via `.context()`
- Automatic backtrace capture

**Disadvantages:**
- No type-level error categorization
- Hard to match on specific errors

**Future Consideration:**
- Custom error enum for recoverable errors
- `anyhow::Error` for unrecoverable errors
- `thiserror` for error enum derives

### Error Recovery
**TUI Mode:**
- Catch errors in event handlers
- Display in status bar
- Don't crash the TUI

**CLI Mode:**
- Print error to stderr
- Exit with non-zero code
- Include actionable error messages

## Deployment & Distribution

### Build Targets
**Primary:**
- Linux x86_64
- macOS (Intel + Apple Silicon)
- Windows x86_64

**Considerations:**
- Static linking for Linux (musl target)
- Universal binary for macOS
- Installer for Windows

### Dependency Management
**External Dependencies:**
- SQLite (bundled via `sqlx` or system library)
- libgit2 (bundled via `git2` or system library)

**Trade-offs:**
- Bundled: Larger binary, guaranteed version
- System: Smaller binary, must be installed

**Recommendation:** Use bundled dependencies for easier distribution

### Release Process
**Gap:** No CI/CD pipeline defined

**Proposed:**
- GitHub Actions for builds
- Cross-compilation for all platforms
- Automated releases on tag push
- Checksum generation for verification

## Future Enhancements

### Short-term (MVP+1)
1. **Configuration file support** - TOML-based config
2. **Authentication** - Gerrit login with credential storage
3. **Full NoteDb parsing** - Comments, votes, labels
4. **Advanced search** - Field-specific filters

### Medium-term
1. **Code review submission** - Vote, comment via API
2. **Multi-repo support** - Work with multiple projects
3. **Custom queries** - Save and reuse search queries
4. **Export capabilities** - CSV, JSON output

### Long-term
1. **Plugin system** - Extensibility for custom workflows
2. **Collaboration features** - Share queries/configurations
3. **Git worktree integration** - Automatic worktree per change
4. **CI/CD integration** - Trigger builds, fetch results

## Known Gaps & Open Questions

### Technical Gaps
1. **NoteDb Format Specification** - Need to reverse-engineer or find docs
2. **Gerrit Authentication** - Which auth method(s) to support?
3. **Configuration Management** - File format, location, precedence
4. **Error Recovery** - Retry logic for network failures
5. **Performance Benchmarks** - What are acceptable limits?
6. **Log Retention** - How long to keep logs? Compression?

### Design Decisions Needed
1. **Multi-tenancy** - Support multiple Gerrit instances?
2. **Change-level vs Patchset-level** - Current model is change-level
3. **Offline Mode** - How much functionality without Gerrit access?
4. **Conflict Resolution** - Auto-merge vs manual intervention for cherry-picks
5. **Notification System** - Desktop notifications for events?

### Integration Questions
1. **Git Hooks** - Should we install any hooks?
2. **Editor Integration** - Vim/Emacs plugins?
3. **Shell Completion** - Bash/Zsh/Fish completion scripts?
4. **System Tray** - Background daemon for continuous sync?

## Success Criteria

### Functional Requirements
- ✅ View list of changes (TUI + CLI)
- ✅ Search changes with fuzzy matching
- ✅ Sync changes from Gerrit
- ✅ Cherry-pick changes
- ⚠️ View change details (partial - needs NoteDb)
- ❌ Submit reviews (not implemented)
- ❌ View comments (not implemented)

### Non-Functional Requirements
- ✅ Responsive TUI (< 100ms render time)
- ✅ Async operations don't block UI
- ✅ Structured logging for debugging
- ✅ Comprehensive test coverage (unit + integration)
- ⚠️ Cross-platform support (need testing)
- ❌ Security hardening (no auth yet)

### User Experience
- ✅ Intuitive CLI interface
- ✅ Keyboard-driven TUI
- ⚠️ Clear error messages (needs refinement)
- ❌ Documentation (needs writing)
- ❌ Example workflows (needs creation)

## Conclusion

This tech stack provides a solid foundation for a Git/Gerrit workflow tool with both TUI and CLI interfaces. The use of Rust with Tokio enables high performance and safety, while ratatui and crossterm provide a responsive terminal UI. The architecture cleanly separates concerns with clear data flows.

Key strengths:
- Async-first design for responsiveness
- Structured concurrency with tokio-scoped
- Local-first with SQLite caching
- Comprehensive observability via tracing
- Strong testing infrastructure

Areas requiring attention:
- Authentication and security
- NoteDb format reverse-engineering
- Configuration management
- Performance optimization for large datasets
- Documentation and user guides

The modular design allows incremental development and testing of each component independently, with clear integration points defined.

---

**Next Steps:**
1. Validate NoteDb format by examining actual Gerrit repository
2. Design authentication flow and credential storage
3. Define configuration file format and precedence
4. Create detailed API design document
5. Develop user documentation and tutorials
