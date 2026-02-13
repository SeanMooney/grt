# Gertty Configuration and UI

**Source project:** gertty
**Source files:** `gertty/config.py`, `gertty/view/`, `gertty/palette.py`, `gertty/keymap.py`, `gertty/app.py`, `gertty/mywid.py`, `gertty/commentlink.py`
**Status:** Draft
**Informs:** `config-system.md`, `tui-design.md`

## Overview

Gertty is a console-based interface to the Gerrit Code Review system, built on Python's urwid terminal UI library. Its architecture combines a YAML-based configuration system with a retained-mode widget tree UI, a view-stack navigation model, and a background sync engine.

The configuration system loads a single YAML file, validates it with a voluptuous schema, and inflates it into a `Config` object that the rest of the application reads. Configuration governs server connections, color palettes, keybindings, dashboards, comment link patterns, and numerous behavioral options.

The UI is structured around a `urwid.MainLoop` that manages a `urwid.Frame` with a header (status bar), footer (breadcrumbs), and a body that holds the current view. Views are full-screen widgets pushed onto and popped from a view stack. Popups (dialogs, help) are rendered as `urwid.Overlay` widgets on top of the current view. Every view implements a `keypress` method that translates key events into commands via the keymap system, and an `interested`/`refresh` pair for reacting to sync events.

The palette system defines named color attributes that urwid uses for styling widgets. The keymap system maps command names to key sequences (including multi-key chords) and provides a tree-based lookup for resolving key presses to commands.

## Configuration

### Config File Format

Gertty uses a single YAML configuration file. The default location is `~/.config/gertty/gertty.yaml` with a fallback to `~/.gertty.yaml`. An explicit path can be provided via the `-c` command-line flag.

The file is loaded with `yaml.safe_load` and validated against a voluptuous schema defined in `ConfigSchema`. The top-level structure has one required key (`servers`) and many optional keys:

```yaml
servers:
  - name: mygerrit
    url: https://review.example.com
    username: jdoe
    password: secret
    git-root: ~/git
    auth-type: digest          # basic, digest, or form
    verify-ssl: true
    ssl-ca-path: /path/to/ca.pem
    dburi: sqlite:///~/.gertty.db
    git-url: https://review.example.com
    log-file: ~/.gertty.log
    lock-file: ~/.gertty.mygerrit.lock
    socket: ~/.gertty.sock

palette: default               # or 'light', or a custom palette name
keymap: default                # or 'vi', or a custom keymap name
palettes: [...]                # custom palette definitions
keymaps: [...]                 # custom keymap definitions
commentlinks: [...]            # regex-based link/text/search replacements
dashboards: [...]              # named change queries bound to keys
reviewkeys: [...]              # one-key review shortcuts
change-list-query: "status:open"
diff-view: side-by-side        # or 'unified'
hide-comments: [...]           # authors whose comments to hide
thread-changes: true           # show parent-child threading in change lists
display-times-in-utc: false
handle-mouse: true
breadcrumbs: true
close-change-on-review: false
change-list-options:
  sort-by: number              # number, updated, last-seen, project
  reverse: false
expire-age: "2 months"
size-column:
  type: graph                  # graph, split-graph, number, disabled
  thresholds: [1, 10, 100, 1000]
```

### Config Layers

Gertty has a two-layer configuration precedence model, not three:

1. **Hardcoded defaults** -- The `Config.__init__` method applies defaults for every optional field using `.get()` with fallback values. For example: `self.diff_view = self.config.get('diff-view', 'side-by-side')`, `self.breadcrumbs = self.config.get('breadcrumbs', True)`, `self.thread_changes = self.config.get('thread-changes', True)`.

2. **User YAML file** -- Values in the YAML file override the hardcoded defaults. The file is validated against the voluptuous schema before any values are read.

Command-line arguments do not override individual config values. The CLI flags (`-p` for palette, `-k` for keymap, `server` positional arg) select which named palette, keymap, or server block to use, but they do not set arbitrary config keys. The only config-path override is `-c` for the file location.

For palettes and keymaps specifically, there is an additional layering: built-in presets (`default`, `light` for palettes; `default`, `vi` for keymaps) are loaded first, then user-defined palettes/keymaps in the config file can either extend existing ones (by reusing the name) or define entirely new ones.

### Server Definitions

The `servers` key is the only required top-level key. It is a list of server objects, each validated against this schema:

```python
server = {
    v.Required('name'): str,        # identifier used for selection
    v.Required('url'): str,         # Gerrit base URL
    v.Required('username'): str,    # HTTP auth username
    'password': str,                # if omitted, prompts interactively
    v.Required('git-root'): str,    # local directory for git clones
    'git-url': str,                 # URL for git operations (defaults to url)
    'verify-ssl': bool,             # defaults to True
    'ssl-ca-path': str,             # custom CA bundle path
    'dburi': str,                   # SQLite URI (defaults to ~/.gertty.db)
    'auth-type': v.Any('basic', 'digest', 'form'),  # defaults to 'digest'
    'log-file': str,                # defaults to ~/.gertty.log
    'lock-file': str,               # defaults to ~/.gertty.<name>.lock
    'socket': str,                  # UNIX socket for IPC
}
```

Multi-server support works by listing multiple server objects. The server is selected at startup either by name (positional CLI argument) or by picking the first entry. Only one server is active per running instance of gertty; a file lock prevents multiple instances for the same server.

When a password is present in the file, gertty enforces `0600` permissions on the config file and exits with an error if the permissions are looser. When no password is provided, it prompts interactively via `getpass`.

SSL settings propagate to both the Python `requests` library (via `REQUESTS_CA_BUNDLE`) and git subprocess calls (via `GIT_SSL_NO_VERIFY` and `GIT_SSL_CAINFO` environment variables).

## Views

Gertty's UI consists of a small set of full-screen views, each implemented as a class that extends `urwid.WidgetWrap` (and often also `mywid.Searchable` for interactive search). Every view follows a consistent pattern:

- A `title` (and often `short_title`) attribute used by the status header and breadcrumbs
- An `interested(event)` method that returns `True` if a sync event is relevant to this view
- A `refresh()` method that re-reads from the database and updates the widget tree
- A `keypress(size, key)` method that translates keys to commands via the keymap
- A `getCommands()` method returning the list of context-specific commands
- A `help()` method returning formatted command descriptions for the help dialog

### Project List View

**File:** `gertty/view/project_list.py`
**Class:** `ProjectListView(urwid.WidgetWrap, mywid.Searchable)`

This is the root view -- the first screen displayed when gertty starts. It shows all Gerrit projects with their unreviewed and open change counts.

**Key features:**
- **Filtering:** Togglable filters for subscribed-only (`L`) and unreviewed-only (`l`) projects
- **Topics:** Projects can be organized into collapsible topic groups. Topics are locally-managed groupings (create, rename, delete, move/copy projects between topics)
- **Subscription management:** Toggle project subscription with `s`; subscribing triggers background sync
- **Columns:** Project name, unreviewed change count, open change count
- **Navigation:** Selecting a project opens a `ChangeListView` scoped to that project

**Row types:** Two distinct row classes -- `ProjectRow` for individual projects and `TopicRow` for topic headers. Each manages its own urwid widget tree with attribute maps for focus/unfocus styling.

**Marking:** Projects can be marked with `%` for batch operations (move/copy to topic, subscribe multiple).

### Change List View

**File:** `gertty/view/change_list.py`
**Class:** `ChangeListView(urwid.WidgetWrap, mywid.Searchable)`

Displays a list of changes matching a query. Used both for project-scoped views (when navigating from the project list) and for search results and dashboards.

**Key features:**
- **Dynamic columns:** The view has required columns (Number, Subject, Updated) and optional columns (Topic, Branch, Size). Optional columns are shown or hidden based on terminal width, recalculated on resize.
- **Sorting:** Sort by number, updated time, or last-seen, with reverse toggle. Sort state is initialized from config but can be changed at runtime.
- **Threading:** When `thread-changes` is enabled in config, related changes are displayed with tree-drawing characters showing parent-child relationships (using a `ThreadStack` to traverse the commit graph).
- **Size visualization:** Configurable via `size-column` config: `graph` (logarithmic bar chart using Unicode block elements), `split-graph` (separate added/removed bars), `number` (threshold-colored count), or `disabled`.
- **Review labels:** Dynamically adds columns for each review category found in the changes, showing the maximum vote with appropriate coloring (positive/negative/max/min).
- **Batch operations:** Mark changes with `%`, then perform bulk review, abandon, restore, or topic/hashtag editing.

**Event interest:** Refreshes when changes are added to the relevant project or when a displayed change is updated.

### Change Detail View

**File:** `gertty/view/change.py`
**Class:** `ChangeView(urwid.WidgetWrap)`

The most complex view, showing full details of a single change.

**Key features:**
- **Metadata display:** Change-Id, owner, project, branch, topic, hashtags, created/updated timestamps, status, and permalink. Many of these are clickable `TextButton` widgets that trigger searches.
- **Revision rows:** Each patchset is rendered as a `RevisionRow` that can be expanded/collapsed. When expanded, shows the file list with added/removed line counts, plus action buttons (Review, Diff, Local Checkout, Local Cherry-Pick, Submit).
- **Message thread:** Change messages are rendered as `ChangeMessageBox` widgets with author styling, timestamps, and inline comment display. Supports reply functionality. Comment links (URLs, search queries) are active within message text.
- **Related changes:** "Depends On" and "Needed By" sections show related changes as clickable buttons.
- **Review workflow:** The `ReviewDialog` popup presents approval categories with radio buttons for each permitted value, a message text area, and Save/Save-and-Submit/Cancel buttons.
- **Navigation between changes:** `n`/`p` keys navigate to next/previous change in the parent change list without going back.
- **Hidden comments:** Comments from configured authors can be toggled visible/hidden.
- **Check results:** Displays CI check results with color-coded status (SUCCESSFUL, FAILED, RUNNING, etc.).

### Diff View

**Files:** `gertty/view/diff.py` (base class), `gertty/view/unified_diff.py`, `gertty/view/side_diff.py`
**Classes:** `BaseDiffView`, `UnifiedDiffView(BaseDiffView)`, `SideDiffView(BaseDiffView)`

The diff view shows file-by-file diffs between patchsets (or between base and a patchset). The default mode is configured via `diff-view` and defaults to `side-by-side`.

**Base class (`BaseDiffView`) provides:**
- **Patchset selection:** A `PatchsetDialog` allows selecting which old/new patchsets to compare. Arrow keys (`<`/`>`) navigate between consecutive patchset pairs.
- **Context expansion:** `DiffContextButton` widgets allow expanding collapsed context regions (expand 10 lines from top, expand all, expand 10 lines from bottom).
- **Inline comments:** Existing comments are rendered inline below the relevant line. Pressing Enter on a diff line opens a comment editor. Draft comments are styled distinctly.
- **Comment association:** Comments are keyed by `{old|new}[draft]-{lineno}-{path}` and matched to diff lines during rendering.
- **File header:** Each file gets a header showing old/new filenames and status.
- **Review key shortcuts:** The diff view supports `reviewkeys` -- pressing a configured key immediately saves a review with predefined approval values.

**Unified diff (`UnifiedDiffView`):**
- Shows old and new lines sequentially, with two line-number columns (old, new) and a single content column
- Added/removed/context lines are styled differently
- Comments appear below the line they reference, indented

**Side-by-side diff (`SideDiffView`):**
- Shows old and new content in parallel columns, each with its own line-number column
- Tab/Shift-Tab switches focus between left and right comment editors
- Comments can be placed on either the old or new side

### Comment View

Gertty does not have a standalone comment view. Comments are handled in two contexts:

1. **Within the Change Detail View:** `ChangeMessageBox` renders change-level messages with their associated inline comments. Each message shows the author, timestamp, message text (with comment link processing), and any inline file comments grouped by file path.

2. **Within the Diff View:** `BaseDiffCommentEdit` and `BaseDiffComment` are base classes for inline comment editing and display. Comments are positioned immediately after the line they reference. Draft comment editors are persistent -- they are created when the user presses Enter on a line and remain in the widget tree until saved or discarded.

Comment text is processed through the `commentlink` system: regex patterns match portions of text and replace them with styled text, clickable links (opening URLs or internal search queries), or both.

## Navigation Model

Gertty uses a **view stack** for navigation, implemented as a `urwid.MonitoredList` stored in `app.screens`.

### Push/Pop Mechanics

- **Push:** `app.changeScreen(widget)` appends the current `frame.body` to `screens` and sets the new widget as the frame body. The status header updates with the new view's `title`.
- **Pop:** `app.backScreen(target_widget=None)` pops the most recent screen from the stack and restores it as the frame body. If `target_widget` is provided, it pops until it finds that specific widget (discarding intermediate views).
- **Clear:** `app.clearHistory()` pops all screens, returning to the root (project list). Triggered by `meta-home` (TOP_SCREEN command).
- **Popups:** `app.popup(widget)` creates a `urwid.Overlay` over the current view and pushes the current body onto the stack. Popups use `backScreen` to dismiss.

### Breadcrumbs

When `breadcrumbs: true` in config (the default), a `BreadCrumbBar` widget in the frame footer shows the navigation trail. It reads `title` or `short_title` from each screen in the stack, truncating to 25 characters. The bar auto-scrolls to keep the current (rightmost) breadcrumb visible by setting the urwid Columns focus position to the last element.

### View Chaining

The typical navigation flow is:

```
ProjectListView -> ChangeListView -> ChangeView -> DiffView
                                         |
                                    ReviewDialog (popup overlay)
```

Each arrow represents a push onto the view stack. `Esc` (PREV_SCREEN) pops back one level. `meta-home` (TOP_SCREEN) clears the stack back to the project list.

Within `ChangeView`, the `n`/`p` keys (NEXT_CHANGE/PREV_CHANGE) walk the parent change list's ordering without pushing/popping -- they replace the current view in place by finding the `ChangeListView` in the stack and querying it for the adjacent change key.

### Refresh and Event System

The sync engine runs in a background thread and communicates with the UI via pipes (`loop.watch_pipe`). When sync results arrive, `app.refresh()` checks if the current view is `interested` in any queued events and calls `refresh()` on the view if so. This event-driven approach avoids unnecessary full-screen redraws.

## Keybindings

### Default Keymap

The keymap system defines named commands as string constants (e.g., `TOGGLE_REVIEWED = 'toggle reviewed'`, `DIFF = 'diff'`) and maps them to key sequences in `DEFAULT_KEYMAP`:

```python
DEFAULT_KEYMAP = {
    PREV_SCREEN: 'esc',
    TOP_SCREEN: 'meta home',
    HELP: ['f1', '?'],
    QUIT: ['ctrl q'],
    CHANGE_SEARCH: 'ctrl o',
    TOGGLE_REVIEWED: 'v',
    TOGGLE_STARRED: '*',
    TOGGLE_HELD: '!',
    REVIEW: 'r',
    DIFF: 'd',
    REFRESH: 'ctrl r',
    # Multi-key sequences:
    SORT_BY_NUMBER: [['S', 'n']],      # press S then n
    SORT_BY_UPDATED: [['S', 'u']],
    NEW_PROJECT_TOPIC: [['T', 'n']],
    # ...
}
```

Commands are scoped by context:
- **Global commands** (defined in `mywid.GLOBAL_HELP`): help, back, top, quit, search, list held, kill/yank (editing)
- **View-specific commands:** Each view's `getCommands()` returns its context-sensitive command list

### Vi Keymap

A built-in `vi` keymap overrides cursor movement with `h/j/k/l` and changes quit to `:q` (multi-key):

```python
VI_KEYMAP = {
    QUIT: [[':', 'q']],
    CURSOR_LEFT: 'h',
    CURSOR_DOWN: 'j',
    CURSOR_UP: 'k',
    CURSOR_RIGHT: 'l',
}
```

### Multi-Key Sequences

The keymap system supports multi-key sequences using a tree data structure (`Key` class). When a key press matches the prefix of a longer sequence, `FURTHER_INPUT` is returned, and the app buffers the input and displays available completions in the status bar. For example, pressing `S` shows `n u s r` (the available second keys for sort commands).

### Customization

Users can define custom keymaps in the config file:

```yaml
keymaps:
  - name: default              # extend the default keymap
    toggle-reviewed: 'v'
    diff: 'd'
  - name: my-custom-keymap     # or define a new one
    quit: 'q'
```

The `KeyMap.update()` method merges new bindings into the existing tree, replacing any conflicting mappings. Command names in the config use hyphens (e.g., `toggle-reviewed`) which are converted to spaces internally.

### Key Resolution

`KeyMap.getCommands(keys)` traverses the key tree for a sequence of key presses and returns all matching commands. If the current position in the tree has children (meaning more input could complete a longer sequence), `FURTHER_INPUT` is appended to the result list. The `App.unhandledInput` method handles global commands, while view-specific commands are handled in each view's `keypress` method.

## Themes and Palettes

### Color Scheme Architecture

Gertty's palette system maps named attributes to urwid color specifications. Each attribute is a tuple of `(foreground, background)` strings using urwid's color names (e.g., `'dark green'`, `'light red'`, `'white,bold'`, `'default,standout'`).

The `DEFAULT_PALETTE` dictionary defines approximately 90 named attributes organized by context:

- **General UI:** `focused`, `header`, `error`, `table-header`, `footer`, `link`, `focused-link`
- **Diff view:** `removed-line`, `removed-word`, `added-line`, `added-word`, `nonexistent`, `context-button`, `trailing-ws`, `line-number`, `search-result`, `draft-comment`, `comment`, `comment-name`
- **Change view:** `change-data`, `change-header`, `revision-name`, `revision-commit`, `revision-comments`, `revision-drafts`, `change-message-name`, `reviewer-name`, `check-SUCCESSFUL`, `check-FAILED`, `state-wip`
- **Project list:** `unreviewed-project`, `subscribed-project`, `unsubscribed-project`, `marked-project`
- **Change list:** `unreviewed-change`, `reviewed-change`, `starred-change`, `held-change`, `marked-change`, `added-graph`, `removed-graph`, plus 8 levels of `line-count-threshold-N`

Nearly every attribute has a `focused-*` counterpart used via urwid's `focus_map` mechanism, which typically adds `standout` (reverse video) to the foreground.

### Built-in Palettes

Two palettes are built in:

1. **`default`** -- Dark terminal background, uses the full `DEFAULT_PALETTE` as-is
2. **`light`** -- A delta from default, overriding approximately 15 attributes to use darker colors suitable for light terminal backgrounds (e.g., `'table-header': ['black,bold', '']`, `'unreviewed-project': ['black', '']`)

### Palette Customization

Users can define custom palettes or extend built-in ones in the config file:

```yaml
palette: my-palette
palettes:
  - name: my-palette
    header: ['white,bold', 'dark blue']
    error: ['light red', '']
    added-line: ['dark green', 'light gray']
```

The `Palette` class initializes with the full default palette and then applies overrides via `update()`. The `getPalette()` method converts the internal dictionary into urwid's expected list-of-tuples format for `MainLoop`.

### Focus Map Pattern

Views consistently use urwid's `AttrMap` with `focus_map` dictionaries to style focused/unfocused states. Each row type defines its own focus map. For example, `ProjectRow`:

```python
project_focus_map = {
    None: 'focused',
    'unreviewed-project': 'focused-unreviewed-project',
    'subscribed-project': 'focused-subscribed-project',
    'unsubscribed-project': 'focused-unsubscribed-project',
    'marked-project': 'focused-marked-project',
}
```

This pattern means the base style is applied via `AttrMap(col, style)` and the focus style is automatically applied by urwid when the widget has focus.

## grt Divergences

### TOML vs YAML Config

Gertty uses YAML with voluptuous schema validation. grt will use TOML, which provides stronger typing at the syntax level (native booleans, integers, arrays, tables) and eliminates the need for some validation. The TOML `[section]` syntax maps naturally to server blocks:

```toml
[servers.mygerrit]
url = "https://review.example.com"
username = "jdoe"
git_root = "~/git"
```

Serde's derive macros with `#[serde(default)]` can replace gertty's manual `.get(key, default)` pattern for defaults.

### ratatui vs urwid (Immediate Mode vs Widget Tree)

Gertty's UI is built on urwid's retained-mode widget tree: widgets are objects that persist across frames, and updates mutate the tree in place (e.g., `self.subject.set_text(...)`, `self.listbox.body.insert(i, row)`). Focus management, scrolling, and input routing are handled by the widget hierarchy.

ratatui uses an immediate-mode rendering model: the application redraws the entire screen (or relevant portions) each frame by calling render functions. State lives in the application, not in widgets. This has significant implications:

- **Simpler state management:** No need to synchronize widget state with application state; the application state *is* the source of truth
- **No widget tree mutation:** Instead of carefully inserting/removing rows from a list walker, just re-render from the current data
- **Explicit focus/scroll tracking:** The application must maintain cursor position, scroll offset, and focus state explicitly, since there is no widget tree to manage it
- **Layout recomputation:** Column widths, text wrapping, etc. are computed each frame rather than cached in widget objects

Gertty's `AttrMap` + `focus_map` pattern for styling focused items would be replaced by conditional style application during rendering in ratatui.

### crossterm vs curses

Gertty uses urwid's curses-based screen backend. grt will use crossterm (via ratatui), which provides:

- Cross-platform support (Windows, macOS, Linux) without curses dependency
- Direct terminal manipulation without ncurses initialization overhead
- Async-compatible event polling that integrates with tokio
- Better Unicode and true-color support

### Potential for Vim-Style Modal Navigation

Gertty's built-in vi keymap is minimal -- it only remaps cursor movement (`hjkl`) and quit (`:q`). The multi-key sequence system (e.g., `S`+`n` for sort-by-number) is closer to emacs chord notation than vim modal editing.

grt could implement a richer modal system:

- **Normal mode:** Vim-like navigation (`j`/`k` for list movement, `gg`/`G` for top/bottom, `/` for search, numeric prefixes for repeat counts)
- **Command mode:** `:` prefix for commands (`:q`, `:search <query>`, `:sort updated`)
- **Insert mode:** Entered when editing text fields (comments, search queries), with `Esc` returning to normal mode

This would be a departure from gertty's flat command space where all keys are context-sensitive but non-modal. The multi-key tree structure from gertty could still be useful for commands like `gg` (go to top) or `dd` (potential batch operation), but within a modal framework.

The `keymap.py` tree-based key resolution system is worth studying for grt's implementation, though it would need to be mode-aware: the same key (`j`) would mean "cursor down" in normal mode but insert a character in insert mode.
