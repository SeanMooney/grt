# git-review Workflow

**Source project:** git-review
**Source files:** `git_review/cmd.py`, `git_review/hooks.py`
**Status:** Draft
**Informs:** `cli-design.md`, `git-operations.md`

## Overview

git-review is a Python CLI tool (`git review`) that wraps the Git push workflow for submitting changes to Gerrit code review. Its primary job is to construct and execute the correct `git push` command targeting Gerrit's magic `refs/for/<branch>` refspec, along with managing the prerequisites that make that push succeed: ensuring a Gerrit remote exists, installing the commit-msg hook that generates Change-Id trailers, optionally rebasing onto the target branch, and validating that the commit set looks reasonable before pushing.

The tool operates as a single-shot command -- it is invoked, performs its work through subprocess calls to git, and exits. There is no persistent daemon or background state. The overall flow of a push invocation is:

1. **Configuration loading** -- Read `.gitreview` file, layer git config overrides on top, determine remote/branch/project.
2. **Remote verification** -- Ensure the configured Gerrit remote exists; create it if missing.
3. **Hook installation** -- Ensure the `commit-msg` hook is present; install it (from a vendored copy, HTTP, or SCP) if missing.
4. **Pre-push custom scripts** -- Run any `pre-review` scripts found in hook directories.
5. **Rebase (optional)** -- Perform a test rebase against `remotes/<remote>/<branch>` to detect merge conflicts.
6. **Commit validation** -- Warn the user if there are zero commits, one commit without a Change-Id, or multiple commits to push.
7. **Push** -- Execute `git push HEAD:refs/for/<branch>` with any push options appended (topic, reviewers, WIP flags, etc.).
8. **Post-push** -- Optionally run `post-review` custom scripts and/or delete the local branch (`--finish`).

git-review also supports non-push operations (downloading a change, cherry-picking, listing open reviews, comparing patchsets), but those are outside the scope of this document.

## Push Workflow

### Branch-to-Ref Mapping

The core of git-review's push is a single `git push` command that maps the local HEAD to a Gerrit magic ref. The ref is constructed as:

```
refs/for/<branch>
```

The `<branch>` value is determined by (in priority order):
1. An explicit branch argument on the command line (`git review mybranch`).
2. The tracked upstream branch, if `--track` is enabled and the current branch tracks a remote branch.
3. The `defaultbranch` value from `.gitreview` (falls back to `master` if unset).

The push command is assembled as a string in `_main()`:

```python
ref = "for"
cmd = ("git %s push --no-follow-tags %s %s HEAD:refs/%s/%s" %
       (color_remote, no_thin, remote, ref, branch))
```

Note that the `ref` variable is always `"for"` -- the older `refs/drafts/` path (used for Gerrit's legacy draft workflow) has been removed from git-review's current codebase.

Push options are appended after a `%` separator using Gerrit's receive-pack syntax:

```python
if push_options:
    cmd += "%" + ",".join(push_options)
```

This produces refspecs like:
```
HEAD:refs/for/main%topic=my-feature,r=reviewer@example.com,wip
```

### Normal Push

A basic push with no extra options is the simplest path:

```
git review main
```

This produces:
```
git push gerrit HEAD:refs/for/main
```

Before pushing, git-review runs `assert_one_change()` which uses `git log HEAD --not --remotes=<remote>` to count how many commits will be pushed. The behavior differs based on the count:

- **Zero commits**: Warns that HEAD already exists on the remote and asks for confirmation.
- **One commit without a hook**: If the commit-msg hook was not installed when the commit was created (i.e., no Change-Id trailer), git-review automatically amends the commit to trigger Change-Id generation: `git commit --amend` with `GIT_EDITOR='true'`.
- **Multiple commits**: Warns the user and asks for confirmation (unless `-y`/`--yes` is given), since submitting multiple commits creates dependent changes in Gerrit.

### Draft/WIP Push

Gerrit's legacy "draft" mode (`refs/drafts/`) is no longer supported. The modern equivalent is Work-In-Progress (WIP), available for Gerrit >= 2.15. The flags are:

- `-w` / `--wip` / `--work-in-progress`: Adds `wip` to push options.
- `-W` / `--ready`: Adds `ready` to push options (transitions a WIP change to ready-for-review).
- `-p` / `--private`: Adds `private` to push options.
- `-P` / `--remove-private`: Adds `remove-private` to push options.

These are mutually exclusive within their groups (wip/ready, private/remove-private). The resulting push command looks like:

```
git push gerrit HEAD:refs/for/main%wip
git push gerrit HEAD:refs/for/main%ready
git push gerrit HEAD:refs/for/main%private
```

### Push with Topic

Topics are set via the `-t`/`--topic` flag. The topic is added as a push option only if it differs from the target branch name:

```python
if topic and topic != branch:
    push_options.append("topic=%s" % topic)
```

By default, the `notopic` config key is `True`, meaning no topic is sent unless explicitly specified. This is a relatively recent change in git-review; the `-T`/`--no-topic` flag is marked as deprecated since it is now the default behavior.

Additional metadata can be pushed alongside:

- `--hashtags TAG [TAG ...]`: Adds `t=TAG` push options for Gerrit hashtags.
- `--reviewers USER [USER ...]`: Adds `r=USER` push options.
- `--cc USER [USER ...]`: Adds `cc=USER` push options.
- `--notify {NONE,OWNER,OWNER_REVIEWERS,ALL}`: Adds `notify=VALUE`.
- `--message TEXT`: Adds `m=URL_ENCODED_TEXT` for the patchset description.

Reviewers, CC, and hashtags are validated for whitespace before inclusion since whitespace in the refspec would break the push:

```python
def assert_valid_whitespace(values, type_name):
    for v in values:
        if re.search(r"\s", v):
            raise MalformedInput(
                "Whitespace not allowed in %s: '%s'" % type_name, v)
```

### Push to Specific Branch

When the user specifies a branch on the command line (`git review feature-x`), it overrides both the `.gitreview` default and the tracked upstream. The `--track` option is also explicitly disabled to prevent confusion:

```python
if options.branch is None:
    branch = config['branch']
else:
    branch = options.branch
    options.track = False
```

When `--track` is enabled and no explicit branch is given, git-review resolves the current branch's upstream tracking reference:

```python
def resolve_tracking(remote, branch):
    tracked_remote, tracked_branch = parse_tracking()
    if tracked_branch:
        return tracked_remote, tracked_branch
    return remote, branch
```

`parse_tracking()` uses `git symbolic-ref -q HEAD` to get the current ref, then `git for-each-ref --format=%(upstream)` to find its upstream. If the upstream is under `refs/remotes/`, it is parsed into remote and branch components.

### Change-Id Regeneration

The `-i`/`--new-changeid` flag strips the existing Change-Id and amends the commit to generate a fresh one:

```python
if options.regenerate:
    run_command(regenerate_cmd,
                GIT_EDITOR="sed -i -e "
                "'/^Change-Id:/d'")
```

This is used when a developer wants to create a new Gerrit change from an existing commit (rather than updating the existing change).

## Change-Id Hook

### Hook Installation

git-review installs the Gerrit `commit-msg` hook automatically when it first runs in a repository. The hook is what generates the `Change-Id: I<hash>` trailer on every commit, which Gerrit uses to correlate commits with review changes.

The installation logic is in `set_hooks_commit_msg()` and has three strategies, tried based on the remote URL type:

1. **No remote URL (default)**: Writes a vendored copy of the hook script directly from `hooks.COMMIT_MSG` -- a Python string literal containing the entire shell script. This is the preferred path because it avoids network dependencies. The vendored copy is taken from Gerrit's source at a specific commit (05d6470, 2024-02-21).

2. **HTTP/HTTPS remote**: Fetches the hook from Gerrit's HTTP endpoint:
   ```python
   hook_url = urljoin(remote_url, '/tools/hooks/commit-msg')
   ```

3. **SSH remote**: Uses `scp` to copy the hook from the Gerrit server:
   ```python
   cmd = ["scp", userhost + ":hooks/commit-msg", scp_target_file]
   ```
   Port is passed via `-P` flag, and the `-O` flag (force SCP protocol) is added if supported by the local `scp` binary.

The `--remote-hook` flag forces strategies 2 or 3 (fetching from the actual Gerrit server) instead of using the vendored copy.

After installation, the hook is also copied into all submodules:

```python
run_command_exc(
    CannotInstallHook,
    "git", "submodule", "foreach",
    'cp -p %s "$(git rev-parse --git-dir)/hooks/"' % target_file)
```

Finally, the hook file's permissions are set to be executable by the owner and by anyone else who has read access.

The hook installation respects `core.hooksPath` from git config:

```python
def git_get_hooks_path(top_dir, git_dir):
    hook_dir = os.path.join(git_dir, "hooks")
    hooks_path_option = git_config_get_value('core', 'hooksPath')
    if hooks_path_option:
        if os.path.isabs(hooks_path_option):
            hook_dir = hooks_path_option
        else:
            hook_dir = os.path.join(top_dir, hooks_path_option)
    return hook_dir
```

### Change-Id Format and Generation

The vendored `commit-msg` hook (from `hooks.py`) is a POSIX shell script. Its logic:

1. **Early exit checks**:
   - If `gerrit.createChangeId` git config is set to `false`, do nothing.
   - If the commit message starts with a `squash!` or `fixup!` prefix (pattern: `^[a-z][a-z]*! `), skip Change-Id generation (unless `gerrit.createChangeId` is `always`).
   - If the commit message file is empty, exit with an error.

2. **Hash generation**: The Change-Id is derived from a hash of the committer identity, the current HEAD ref hash, and the commit message content:
   ```sh
   random=$({ git var GIT_COMMITTER_IDENT ; echo "$refhash" ; cat "$1"; } | git hash-object --stdin)
   ```
   The resulting Change-Id has the format `I<40-hex-chars>` (e.g., `I1a2b3c4d...`).

3. **Review URL mode**: If `gerrit.reviewUrl` is configured, the hook generates a `Link:` trailer instead of a `Change-Id:` trailer, with the value being a URL like `<reviewUrl>/id/I<hash>`.

4. **Idempotency**: If a Change-Id (or Link) trailer already exists in the commit message, the hook exits without modification:
   ```sh
   if git interpret-trailers --parse < "$1" | grep -q "^$token: $pattern$" ; then
     exit 0
   fi
   ```

5. **Trailer insertion**: The hook uses `git interpret-trailers` to insert the Change-Id before any `Signed-off-by` trailers. It uses a sentinel technique to ensure proper ordering: it inserts a temporary `Signed-off-by: SENTINEL` line, then inserts the Change-Id as if it were a `Signed-off-by`, then strips both the sentinel and the `Signed-off-by:` prefix from the Change-Id line using `sed`. This approach avoids relying on `--where` and `--in-place` options that were only available in later git versions.

### Validation on Push

Before pushing, `assert_one_change()` checks whether the commit-msg hook was present when the commit was made. If there is exactly one commit to push and the hook was not installed at commit time, git-review automatically amends the commit to trigger Change-Id generation:

```python
if output_lines == 1 and not have_hook:
    printwrap("Your change was committed before the commit hook was "
              "installed. Amending the commit to add a gerrit change id.")
    run_command("git commit --amend", GIT_EDITOR='true')
```

## Rebase Handling

Rebase is controlled by three mutually exclusive flags:

| Flag | Behavior |
|------|----------|
| (default) | Test rebase, then undo it before pushing |
| `-R` / `--no-rebase` | Skip rebase entirely |
| `-F` / `--force-rebase` | Rebase and keep the result (push the rebased commits) |
| `-K` / `--keep-rebase` | If rebase has conflicts, leave the incomplete rebase in the working tree instead of aborting |

The default rebase behavior (`config['rebase']`, which defaults to `"1"`) is a **test rebase**: git-review rebases onto the remote tracking branch to detect merge conflicts, but then undoes the rebase before actually pushing. This means the push sends the original (non-rebased) commits. The intent is conflict detection, not commit rewriting.

The rebase function `rebase_changes()` follows this sequence:

1. **Update remote**: `git remote update <remote>` to fetch the latest state of the target branch.

2. **Save HEAD**: Records the current HEAD SHA so it can be restored later via `undo_rebase()`:
   ```python
   _orig_head = run_command("git rev-parse HEAD")
   ```

3. **Check for dirty state**: Runs `git diff --ignore-submodules --quiet` for both staged and unstaged changes. If either is dirty, the operation is aborted with a message asking the user to commit or stash first. This is explicitly done to prevent silent data loss when `rebase.autostash` is configured.

4. **Verify target branch exists**: Checks that `refs/remotes/<remote>/<branch>` exists via `git show-ref --quiet --verify`. If not, suggests using `-R` to skip rebase.

5. **Execute rebase**: Runs `git rebase --rebase-merges <remote_branch>` (or `--preserve-merges` for git < 2.18). The `GIT_EDITOR` is set to `true` to suppress interactive editing.

6. **Handle failure**: On conflict, the behavior depends on flags:
   - Default: Prints a message about the conflict, calls `abort_rebase()` (`git rebase --abort`), and exits with status 1.
   - With `-K`/`--keep-rebase`: Leaves the partial rebase in the working tree so the user can resolve conflicts and then run `git review` again.

7. **Undo (default path)**: If rebase succeeds and `--force-rebase` was not set, `undo_rebase()` runs `git reset --hard <orig_head>` to restore the pre-rebase state. The push then sends the original commits.

The `--force-rebase` (`-F`) flag skips the undo step, so the rebased commits are what actually gets pushed.

## Configuration

### Git Config Integration

git-review reads from standard git configuration using `git config --get`. The `Config` class intercepts every key access and checks git config first:

```python
def __getitem__(self, key):
    """Let 'git config --get' override every Config['key'] access"""
    value = git_config_get_value('gitreview', key)
    if value is None:
        value = self.config[key]
    return value
```

This means any `.gitreview` option can be overridden via `git config --global gitreview.<key> <value>` or the repository-local equivalent. The `gitreview.*` config keys include:

| Git Config Key | Purpose | Default |
|----------------|---------|---------|
| `gitreview.remote` | Name of the Gerrit remote | `gerrit` |
| `gitreview.branch` | Target branch for push | `master` |
| `gitreview.rebase` | Whether to test-rebase before push | `1` (true) |
| `gitreview.track` | Use tracked upstream branch | `0` (false) |
| `gitreview.notopic` | Suppress auto-topic | `True` |
| `gitreview.usepushurl` | Use push URL on existing remote instead of creating a new remote | `0` (false) |
| `gitreview.username` | Username for Gerrit remote URL construction | (system username) |
| `gitreview.scheme` | URL scheme for remote | `ssh` |
| `gitreview.hostname` | Gerrit server hostname | (none -- required) |
| `gitreview.port` | Gerrit server port | `None` (defaults to 29418 for SSH) |
| `gitreview.project` | Gerrit project name | (none -- required) |

Additionally, git-review reads several non-`gitreview.*` git config keys:

- `core.hooksPath` -- custom hooks directory
- `color.review` -- color output preference
- `remote.<name>.url` / `remote.<name>.pushurl` -- remote URL resolution
- `url.<base>.insteadOf` / `url.<base>.pushInsteadOf` -- URL rewriting
- `gerrit.createChangeId` -- controls commit-msg hook behavior (`false`, `always`, or default)
- `gerrit.reviewUrl` -- if set, the hook generates `Link:` trailers instead of `Change-Id:`
- `http.sslVerify` -- SSL verification for HTTP requests

### .gitreview File

The `.gitreview` file is an INI-format configuration file placed at the repository root. It uses a single `[gerrit]` section. The file is parsed by Python's `configparser` module:

```ini
[gerrit]
host=review.example.com
port=29418
project=my-project.git
defaultbranch=main
defaultremote=gerrit
defaultrebase=1
track=0
notopic=true
usepushurl=0
scheme=ssh
```

The mapping between `.gitreview` option names and internal config keys is defined in `load_config_file()`:

| .gitreview Key | Internal Config Key | Description |
|----------------|-------------------|-------------|
| `host` | `hostname` | Gerrit server hostname |
| `port` | `port` | Gerrit server port |
| `project` | `project` | Gerrit project name |
| `defaultbranch` | `branch` | Default target branch |
| `defaultremote` | `remote` | Remote name to use |
| `defaultrebase` | `rebase` | Rebase before push (1/0) |
| `track` | `track` | Follow tracked branch |
| `notopic` | `notopic` | Suppress auto-topic |
| `usepushurl` | `usepushurl` | Use push URL mode |
| `scheme` | `scheme` | URL scheme (ssh/http/https) |

The `.gitreview` file is loaded from `<top_dir>/.gitreview` where `top_dir` is determined by `git rev-parse --show-toplevel`.

### Config Precedence

Configuration values are resolved in this order (highest priority first):

1. **Command-line arguments** -- Flags like `-r <remote>`, `-t <topic>`, explicit branch argument.
2. **Git config** (`gitreview.*` section) -- Checked on every `Config[key]` access via `git config --get gitreview.<key>`.
3. **`.gitreview` file** -- Per-repository INI file at the repo root.
4. **Global/system config files** -- `~/.config/git-review/git-review.conf` and `/etc/git-review/git-review.conf` (deprecated; produces a warning).
5. **Hardcoded defaults** -- The `DEFAULTS` dict:
   ```python
   DEFAULTS = dict(scheme='ssh', hostname=False, port=None, project=False,
                   branch='master', remote="gerrit", rebase="1",
                   track="0", usepushurl="0", notopic=True)
   ```

The precedence is implemented through layering: `Config.__init__()` starts with `DEFAULTS`, then overlays options from config files. Each `Config[key]` access then checks `git config --get gitreview.<key>` first, falling back to the file-based value. Command-line arguments override everything at the call site in `_main()`.

A `LOCAL_MODE` environment variable (`GITREVIEW_LOCAL_MODE`) restricts git config reads to only the repository-local config (using `git config -f <git_dir>/config`), skipping global and system configs.

## Error Handling

git-review uses a hierarchical exception system rooted in `GitReviewException`:

```
GitReviewException (EXIT_CODE = 1)
├── CommandFailed
│   ├── GitDirectoriesException (EXIT_CODE = 70)
│   ├── GitConfigException (EXIT_CODE = 128)
│   ├── CannotInstallHook (EXIT_CODE = 2)
│   ├── CustomScriptException (EXIT_CODE = 71)
│   ├── CannotQueryOpenChangesets (EXIT_CODE = 32)
│   ├── CannotQueryPatchSet (EXIT_CODE = 34)
│   ├── PatchSetGitFetchFailed (EXIT_CODE = 37)
│   ├── CheckoutNewBranchFailed (EXIT_CODE = 64)
│   ├── CheckoutExistingBranchFailed (EXIT_CODE = 65)
│   ├── ResetKeepFailed (EXIT_CODE = 66)
│   ├── SetUpstreamBranchFailed (EXIT_CODE = 67)
│   ├── SymbolicRefFailed (EXIT_CODE = 68)
│   ├── ForEachRefFailed (EXIT_CODE = 69)
│   └── DeleteBranchFailed (EXIT_CODE = 68)
├── ChangeSetException
│   ├── CannotParseOpenChangesets (EXIT_CODE = 33)
│   ├── ReviewInformationNotFound (EXIT_CODE = 35)
│   ├── ReviewNotFound (EXIT_CODE = 36)
│   └── PatchSetNotFound (EXIT_CODE = 38)
├── GerritConnectionException (EXIT_CODE = 40)
├── BranchTrackingMismatch (EXIT_CODE = 70)
└── MalformedInput (EXIT_CODE = 3)
```

**`CommandFailed`** is the most important exception class. It captures the full context of a failed subprocess call:

```python
class CommandFailed(GitReviewException):
    def __init__(self, *args):
        (self.rc, self.output, self.argv, self.envp) = args

    def __str__(self):
        return """
The following command failed with exit code %(rc)d
    "%(argv)s"
-----------------------
%(output)s
-----------------------""" % self.quickmsg
```

Each subclass has a unique `EXIT_CODE` and a descriptive docstring that serves as the error message. The `run_command_exc()` function wraps subprocess execution and raises the appropriate exception on non-zero exit:

```python
def run_command_exc(klazz, *argv, **env):
    (rc, output) = run_command_status(*argv, **env)
    if rc:
        raise klazz(rc, output, argv, env)
    return output
```

The top-level `main()` function catches all `GitReviewException` subclasses and converts them to exit codes:

```python
def main():
    try:
        _main()
    except GitReviewException as e:
        print(e)
        sys.exit(e.EXIT_CODE)
```

User-facing messages are printed through `printwrap()` (wraps text at terminal width) and `warn()` (prepends "WARNING:"). Interactive confirmation prompts are used at two points: when pushing zero commits (commit already on remote) and when pushing multiple commits.

All subprocess calls force `LANG=C` and `LANGUAGE=C` environment variables to ensure consistent English output from git, which is important for parsing output:

```python
newenv = os.environ.copy()
newenv['LANG'] = 'C'
newenv['LANGUAGE'] = 'C'
```

## grt Divergences

The following areas represent concrete differences between git-review's approach and how grt will handle the same concerns:

**gix library vs. subprocess calls.** git-review calls every git operation via `subprocess.Popen`, parsing the text output. grt will use gix (gitoxide), a pure-Rust git implementation, for operations where possible (reading config, resolving refs, inspecting the working tree, examining commit graphs). This eliminates the need to force `LANG=C` for output parsing, avoids the overhead of process spawning, removes the C dependency on libgit2, and provides structured data instead of text that must be string-matched. However, `git push` to Gerrit's magic refs may still require shelling out to `git`, since gix's push support may not handle Gerrit's custom receive-pack options (the `%topic=...` syntax appended to refspecs). All gix calls are wrapped in `tokio::task::spawn_blocking` since gix's API is blocking.

**TUI integration.** git-review's confirmation prompts (`input("Type 'yes' to confirm")`) and `printwrap()` output are designed for a line-oriented CLI. grt's TUI (ratatui) will present these as interactive elements: multi-commit push warnings can be shown as scrollable lists with accept/cancel keybindings; rebase conflict resolution can present a status view showing conflicted files. The TUI also enables progressive display of long-running operations (rebase progress, push progress) rather than blocking on a subprocess.

**Batch operations.** git-review operates on a single branch/change at a time. grt can batch operations: pushing multiple branches to review simultaneously, rebasing a stack of dependent changes, or applying topics across a set of related changes. The SQLite cache enables grt to know the current state of all local branches and their corresponding Gerrit changes without querying the server each time.

**Configuration model.** git-review's `Config` class does a git-config subprocess call on every key access, which is simple but involves repeated process spawning. grt will load all configuration at startup into a structured `Config` type, reading `.gitreview` (or a grt-specific config file), git config, and defaults in a single pass. The layered precedence model (CLI > git config > file > defaults) will be preserved.

**Hook management.** git-review installs hooks by either writing a vendored shell script, fetching via HTTP, or copying via SCP. grt can take a simpler approach: since it will always ensure Change-Id trailers are present before pushing (either by checking commits directly via gix or by running the hook logic natively in Rust), the external shell hook becomes less critical. grt may still install the hook for compatibility with manual `git commit` usage outside of grt, but the push path will not depend on it.

**Error handling.** git-review maps errors to numeric exit codes (2, 3, 32-40, 64-71, 128) with text output. grt will use Rust's `Result`/`Error` types with structured error enums, and the TUI can display errors contextually (inline in the relevant panel) rather than as terminal text. The CLI mode will still use exit codes for scripting compatibility, but the codes and messages will be grt's own, not git-review's.

**Rebase strategy.** git-review's default "test rebase then undo" pattern (rebase to check for conflicts, then `git reset --hard` back to the original HEAD) is unusual and potentially confusing. grt will likely default to either no rebase or a rebase-and-keep model, with conflict detection handled through gix's merge analysis APIs rather than actually performing and undoing a rebase.

**URL rewriting.** git-review manually parses `url.*.insteadOf` and `url.*.pushInsteadOf` from `git config --list` output and applies rewrite rules in Python. grt can delegate this to gix's remote URL resolution, which handles these rewrites natively.
