# git-review Gerrit API Usage

**Source project:** git-review
**Source files:** `git_review/cmd.py`
**Status:** Draft
**Informs:** `gerrit-client.md`

## Overview

git-review interacts with Gerrit through a dual-protocol approach: SSH commands and HTTP/REST API calls. The protocol used for any given operation is determined by the remote URL scheme -- if the remote URL starts with `http://` or `https://`, git-review uses the HTTP/REST path; otherwise, it assumes SSH. This decision is made dynamically at the call site, with a dispatcher function selecting the appropriate implementation:

```python
def query_reviews(remote_url, project=None, branch=None, change=None,
                  current_patch_set=True, exception=CommandFailed,
                  parse_exc=Exception):
    if remote_url.startswith('http://') or remote_url.startswith('https://'):
        query = query_reviews_over_http
    else:
        query = query_reviews_over_ssh
    return query(remote_url, ...)
```

The two protocols are not used simultaneously -- a given repository has either an SSH or HTTP remote, and all API interactions use that single protocol. The actual `git push` to Gerrit always goes through git's own transport (SSH or HTTP depending on the remote URL), but the API queries (listing changes, fetching change metadata, version detection) use a separate code path that either shells out to SSH or makes HTTP requests.

git-review's HTTP layer uses the `requests` library (a synchronous HTTP client). Its SSH layer shells out to the system `ssh` binary via `subprocess.Popen`. There is no SSH library usage -- all SSH operations are external process invocations.

The overall API surface that git-review touches is narrow: it queries for open changes, fetches change details for downloading specific patchsets, and retrieves the commit-msg hook. It does not use the Gerrit API for submitting changes (that goes through `git push`), posting comments, managing labels, or any other write operation beyond the push.

## SSH Protocol

git-review uses SSH as a transport for Gerrit's command-line interface. Gerrit exposes an SSH server (typically on port 29418) that accepts commands in the form `gerrit <command> [args]`. git-review invokes these commands by spawning an SSH process.

### Commands Used

git-review uses a single Gerrit SSH command: `gerrit query`. This command is used to search for open changes, retrieve change metadata, and look up specific changes for download.

The `query_reviews_over_ssh()` function constructs and executes the query:

```python
def query_reviews_over_ssh(remote_url, project=None, branch=None, change=None,
                           current_patch_set=True, exception=CommandFailed,
                           parse_exc=Exception):
    (hostname, username, port, project_name) = \
        parse_gerrit_ssh_params_from_git_url(remote_url)

    if change:
        if current_patch_set:
            query = "--current-patch-set change:%s" % change
        else:
            query = "--patch-sets change:%s" % change
    else:
        query = "project:%s status:open" % project_name
        if branch:
            query += ' branch:%s' % branch

    output = run_command_exc(
        exception,
        os.environ.get("GIT_SSH", "ssh"), "-x" + port_data, userhost,
        "gerrit", "query",
        "--format=JSON %s" % query)
```

The queries fall into two categories:

1. **Listing open changes**: When no specific change number is provided, git-review queries for all open changes in the project, optionally filtered by branch:
   ```
   gerrit query --format=JSON project:<project> status:open [branch:<branch>]
   ```

2. **Fetching a specific change**: When a change number is provided, git-review queries for that specific change with patchset information:
   ```
   gerrit query --format=JSON --current-patch-set change:<number>
   gerrit query --format=JSON --patch-sets change:<number>
   ```
   The `--current-patch-set` flag fetches only the latest patchset's ref, while `--patch-sets` fetches all patchset refs (used when the user requests a specific patchset number).

The SSH output format is JSON-per-line: each line of output is a separate JSON object. git-review parses this line by line, filtering out the final summary line (which has a `"type"` key):

```python
changes = []
for line in output.split("\n"):
    if line[0] == "{":
        try:
            data = json.loads(line)
            if "type" not in data:
                changes.append(data)
        except Exception:
            if VERBOSE:
                print(output)
```

The `"type"` filter discards Gerrit's query statistics line (e.g., `{"type":"stats","rowCount":5,"runTimeMilliseconds":42}`), keeping only the actual change records.

git-review does not use other Gerrit SSH commands such as `gerrit review` (for posting reviews/votes), `gerrit ls-projects`, `gerrit version`, or `gerrit set-reviewers`. It also does not use SSH for the push operation itself -- that goes through `git push`, which uses git's own SSH transport.

Additionally, git-review uses `scp` (not an SSH command per se, but related) to fetch the commit-msg hook from the Gerrit server when the remote uses the SSH scheme:

```python
cmd = ["scp", userhost + ":hooks/commit-msg", scp_target_file]
if port is not None:
    cmd.insert(1, "-P%s" % port)
if "O" in scp_opts:
    cmd.insert(1, "-O")
```

### Connection Management

git-review does not implement any SSH connection management of its own. Each SSH invocation is a fresh subprocess call:

```python
output = run_command_exc(
    exception,
    os.environ.get("GIT_SSH", "ssh"), "-x" + port_data, userhost,
    "gerrit", "query", ...)
```

Key aspects of SSH connection handling:

**SSH binary selection**: git-review respects the `GIT_SSH` environment variable, falling back to the bare `ssh` command if unset. This allows users to specify alternate SSH implementations or wrapper scripts:
```python
os.environ.get("GIT_SSH", "ssh")
```

**Port configuration**: The SSH port is extracted from the remote URL. If no port is specified and the scheme is SSH, git-review defaults to Gerrit's standard port 29418:
```python
if port is None and scheme == 'ssh':
    port = 29418
```

The port is passed to SSH as part of a combined flag string (`-x` disables X11 forwarding, and the port is appended with `p`):
```python
port_data = "p%s" % port if port is not None else ""
# Results in: ssh -xp29418 user@host gerrit query ...
```

**Username handling**: The SSH username is extracted from the remote URL. If the URL contains `user@host`, that username is used. If not, SSH's own default username resolution applies (typically the current system user, or whatever is configured in `~/.ssh/config`).

**No SSH multiplexing**: git-review does not configure SSH `ControlMaster`, `ControlPath`, or `ControlPersist` options. Each `gerrit query` invocation opens and closes a fresh SSH connection. If the user has SSH multiplexing configured in their `~/.ssh/config`, it will be picked up automatically, but git-review does not manage it.

**No SSH key management**: git-review does not specify which SSH key to use. It relies entirely on the SSH agent, `~/.ssh/config` `IdentityFile` directives, or SSH's default key search order. There is no `-i` flag passed to SSH.

**No SSH keepalive or timeout**: git-review does not set `ServerAliveInterval`, `ConnectTimeout`, or any other timeout-related SSH options. Long-running queries that stall will block until the SSH process terminates on its own (or is killed by the OS).

**SCP-style URL parsing**: git-review handles two URL formats for SSH remotes. Standard URLs (`ssh://user@host:port/project`) are parsed with Python's `urlparse`. SCP-style addresses (`user@host:project`) are parsed with string splitting. The `parse_gerrit_ssh_params_from_git_url()` function handles both:

```python
if "://" in git_url:
    parsed_url = urlparse(git_url)
    # ... extract from parsed URL
else:
    # Handle SCP-style addresses
    (hostname, path) = git_url.split(":", 1)
    if "@" in hostname:
        (username, hostname) = hostname.split("@", 1)
```

## HTTP/REST Protocol

git-review uses Gerrit's REST API over HTTP/HTTPS for querying changes and fetching the commit-msg hook. The HTTP layer is built on the Python `requests` library.

### Endpoints Used

git-review calls two Gerrit HTTP endpoints:

**1. `/changes/` -- Query changes**

This is the primary REST endpoint, used for listing open changes and fetching specific change details. The `query_reviews_over_http()` function constructs the URL by stripping the project path from the remote URL to derive the Gerrit root URL, then appending `changes/`:

```python
if project:
    clean_url = os.path.splitext(remote_url)[0]
    clean_project = os.path.splitext(project)[0]
    if clean_url.endswith(clean_project):
        remote_url = clean_url[:-len(clean_project)]
url = urljoin(remote_url, 'changes/')
```

The query parameters vary by use case:

- **Listing open changes**: `GET /changes/?q=project:<name>+status:open[+branch:<branch>]`
  ```python
  query = 'project:%s status:open' % project_name
  if branch:
      query += ' branch:%s' % branch
  params = urlencode({'q': query})
  url += '?' + params
  ```

- **Fetching a specific change (latest patchset)**: `GET /changes/?q=<change>&o=CURRENT_REVISION`
  ```python
  url += '?q=%s&o=CURRENT_REVISION' % change
  ```

- **Fetching a specific change (all patchsets)**: `GET /changes/?q=<change>&o=ALL_REVISIONS`
  ```python
  url += '?q=%s&o=ALL_REVISIONS' % change
  ```

The `o=CURRENT_REVISION` and `o=ALL_REVISIONS` are Gerrit query options that include revision (patchset) details in the response, which git-review needs to extract the fetch refs.

**JSON response handling**: Gerrit's REST API prepends a magic prefix (`)]}'\n`) to JSON responses as an XSSI protection measure. git-review strips this by skipping the first 4 characters:

```python
reviews = json.loads(request.text[4:])
```

**Response normalization**: The HTTP response format differs from the SSH format. git-review normalizes the HTTP response to match the SSH output structure so that downstream code can handle both identically:

```python
for review in reviews:
    review["number"] = str(review.pop("_number"))
    if "revisions" not in review:
        continue
    patchsets = {}
    for key, revision in review["revisions"].items():
        fetch_value = list(revision["fetch"].values())[0]
        patchset = {"number": str(revision["_number"]),
                    "ref": fetch_value["ref"]}
        patchsets[key] = patchset
    review["patchSets"] = patchsets.values()
    review["currentPatchSet"] = patchsets[review["current_revision"]]
```

This normalization maps `_number` to `number` (as a string), extracts fetch refs from the nested `revisions` structure, and builds `patchSets` and `currentPatchSet` fields that match the SSH query output format.

**2. `/tools/hooks/commit-msg` -- Fetch commit-msg hook**

When the remote URL is HTTP/HTTPS, the commit-msg hook is fetched from Gerrit's tools endpoint:

```python
hook_url = urljoin(remote_url, '/tools/hooks/commit-msg')
res = run_http_exc(CannotInstallHook, hook_url, stream=True)
with open(target_file, 'wb') as f:
    for x in res.iter_content(1024):
        f.write(x)
```

The hook is streamed in 1024-byte chunks and written to disk. This endpoint is served by Gerrit without authentication.

### Authentication Methods

HTTP authentication in git-review is handled by the `run_http_exc()` function, which implements a two-step authentication strategy:

**Step 1: Unauthenticated request.** The initial request is made without credentials. If the server returns HTTP 401, git-review falls back to git credentials.

**Step 2: git credential fallback.** On a 401 response, git-review invokes `git credential fill` to obtain credentials, then retries the request with HTTP Basic authentication:

```python
def run_http_exc(klazz, url, **env):
    try:
        res = requests.get(url, **env)
        if res.status_code == 401:
            creds = git_credentials(url)
            if creds:
                env['auth'] = creds
                res = requests.get(url, **env)
    except klazz:
        raise
    except Exception as err:
        raise klazz(255, str(err), ('GET', url), env)
    if not 200 <= res.status_code < 300:
        raise klazz(http_code_2_return_code(res.status_code),
                    res.text, ('GET', url), env)
    return res
```

The `git_credentials()` function uses git's credential helper system:

```python
def git_credentials(url):
    cmd = 'git', 'credential', 'fill'
    stdin = 'url=%s' % url
    rc, out = run_command_status(*cmd, stdin=stdin)
    if rc:
        return None
    data = dict(line.split('=', 1) for line in out.splitlines())
    return data['username'], data['password']
```

This leverages whatever credential helpers the user has configured (e.g., `credential.helper=store`, `credential.helper=cache`, OS keychain helpers). The credentials are returned as a `(username, password)` tuple and passed to `requests.get()` as the `auth` parameter, which applies HTTP Basic authentication.

**SSL verification**: For HTTPS URLs, git-review checks two sources for SSL verification preferences:

```python
if url.startswith("https://") and "verify" not in env:
    if "GIT_SSL_NO_VERIFY" in os.environ:
        env["verify"] = False
    else:
        verify = git_config_get_value("http", "sslVerify", as_bool=True)
        env["verify"] = verify != 'false'
```

If `GIT_SSL_NO_VERIFY` is set in the environment, SSL verification is disabled. Otherwise, `http.sslVerify` from git config is checked (it must be explicitly `false` to disable verification; the default is enabled).

## API Version Detection

git-review does **not** perform explicit Gerrit API version detection. There is no call to `/config/server/version` or `gerrit version` (over SSH) to determine the Gerrit server version.

Instead, git-review adapts to Gerrit version differences through a feature-flag approach baked into the command-line options. Features that require specific Gerrit versions are documented in the CLI help text but not programmatically validated:

- Work-in-progress (`--wip`, `--ready`): The help text notes "Gerrit versions >= 2.15" but the code does not check the server version before sending these push options. If the server does not support them, the push will fail with a server-side error.

- Private changes (`--private`, `--remove-private`): Similarly documented as requiring "Gerrit versions >= 2.15" with no runtime version check.

The push options (topic, wip, ready, private, reviewers, hashtags, notify, message) are all sent as Gerrit receive-pack options appended to the refspec. Whether the server accepts or rejects them is left to the server's own validation. git-review does not attempt to pre-validate option compatibility.

The HTTP REST API responses are handled with minimal version sensitivity. The response normalization code in `query_reviews_over_http()` accesses fields like `_number`, `revisions`, `current_revision`, and `fetch` -- these are stable fields in Gerrit's REST API that have been present since at least Gerrit 2.8 (when the REST API was introduced). The code does not branch on API version.

The SSH `gerrit query` output format (`--format=JSON`) has also been stable across Gerrit versions, so no version-specific handling is needed there either.

## Authentication

### HTTP Basic/Digest

git-review uses HTTP Basic authentication via the `requests` library's `auth` parameter. When a request returns HTTP 401, git-review obtains credentials through git's credential helper system and retries with `auth=(username, password)`:

```python
creds = git_credentials(url)
if creds:
    env['auth'] = creds
    res = requests.get(url, **env)
```

The `requests` library's `auth` parameter defaults to HTTP Basic authentication (base64-encoded `username:password` in the `Authorization` header). git-review does not explicitly handle HTTP Digest authentication -- if a server requires Digest auth, the `requests` library would need additional configuration that git-review does not provide. In practice, Gerrit servers use HTTP Basic auth (often with a generated HTTP password distinct from the user's account password).

There is no support for token-based authentication (e.g., Bearer tokens, OAuth), API keys, or other modern authentication schemes. The authentication is strictly limited to what `git credential fill` returns plus HTTP Basic.

### SSH Keys

git-review does not manage SSH keys directly. It delegates all SSH key handling to the system SSH client. The SSH binary is invoked without any `-i` (identity file) flag:

```python
os.environ.get("GIT_SSH", "ssh"), "-x" + port_data, userhost,
"gerrit", "query", ...
```

SSH key selection relies on:

1. **SSH agent**: If `ssh-agent` is running and has keys loaded, SSH will use them automatically.
2. **`~/.ssh/config` IdentityFile directives**: Users can configure per-host key selection in their SSH config.
3. **Default key search order**: SSH tries `~/.ssh/id_rsa`, `~/.ssh/id_ecdsa`, `~/.ssh/id_ed25519`, etc. in order.
4. **`GIT_SSH` environment variable**: If set, the specified program is used instead of `ssh`, which could be a wrapper script that adds key-related flags.

For remote URL construction, git-review determines the SSH username from several sources:

```python
username = git_config_get_value("gitreview", "username")
if not username:
    username = getpass.getuser()
```

If connecting with that username fails, git-review interactively prompts the user for their Gerrit username:

```python
print("Could not connect to gerrit.")
username = input("Enter your gerrit username: ")
```

### .netrc Support

git-review does not directly parse or use `.netrc` files. However, `.netrc` support is available indirectly through two mechanisms:

1. **`requests` library**: The Python `requests` library can pick up credentials from `~/.netrc` when no explicit `auth` parameter is provided. Since git-review's initial (unauthenticated) request uses `requests.get(url)` without `auth`, the `requests` library may apply `.netrc` credentials if they exist for the target host.

2. **Git credential helpers**: If the user has a credential helper that reads `.netrc` (such as `git-credential-netrc`), it will be invoked through git-review's `git credential fill` call on a 401 response.

### Cookie-based Auth

git-review does not implement cookie-based authentication. The `requests` library has session/cookie support, but git-review does not use `requests.Session()` -- each HTTP request is a standalone `requests.get()` call, so no cookies are persisted between requests.

This is a notable difference from gertty, which supports Gerrit's cookie-based authentication (the `.gerritcookies` / `HTTPCookie` mechanism used by `gerrit-cookies` tools). For Gerrit instances that require cookie auth (common in corporate environments behind SSO), git-review's HTTP path may not work, and users would need to use the SSH protocol instead.

## Error Handling

### HTTP Error Handling

The `run_http_exc()` function is the central error handler for all HTTP requests. It handles errors at three levels:

**Network/connection errors**: Any exception during the `requests.get()` call (connection refused, DNS failure, timeout) is caught and wrapped in the provided exception class with a synthetic return code of 255:

```python
except Exception as err:
    raise klazz(255, str(err), ('GET', url), env)
```

**HTTP status code errors**: Any non-2xx response is treated as an error. The HTTP status code is transformed into a system exit code using a modular arithmetic formula that maps HTTP codes into the 1-255 range:

```python
def http_code_2_return_code(code):
    """Tranform http status code to system return code."""
    return (code - 301) % 255 + 1
```

This produces mappings like: 301 -> 1, 302 -> 2, ..., 400 -> 100, 401 -> 101, 404 -> 104, 500 -> 200, etc. The response body is included in the exception as the output string:

```python
if not 200 <= res.status_code < 300:
    raise klazz(http_code_2_return_code(res.status_code),
                res.text, ('GET', url), env)
```

**401 retry**: As described in the authentication section, a 401 response triggers a credential lookup and a single retry. If the retry also fails (with any non-2xx status), the error is raised normally.

### SSH Error Handling

SSH command errors are handled through the `run_command_exc()` mechanism. A non-zero exit code from the SSH process raises the provided exception class with the full context (exit code, output, command, environment):

```python
def run_command_exc(klazz, *argv, **env):
    (rc, output) = run_command_status(*argv, **env)
    if rc:
        raise klazz(rc, output, argv, env)
    return output
```

For SSH query operations specifically, the exception classes are:

- `CannotQueryOpenChangesets` (EXIT_CODE 32): Raised when the SSH query command fails (connection refused, authentication failure, command not found).
- `CannotQueryPatchSet` (EXIT_CODE 34): Raised when querying a specific change fails.

### JSON Parsing Errors

Both the HTTP and SSH query paths have separate exception handling for JSON parsing failures:

```python
except Exception as err:
    raise parse_exc(err)
```

The `parse_exc` parameter allows callers to specify the appropriate exception class. The two parsing exception classes are:

- `CannotParseOpenChangesets` (EXIT_CODE 33): Cannot parse the JSON from a general change listing.
- `ReviewInformationNotFound` (EXIT_CODE 35): Cannot extract review information from the query response.

### Retry Logic

git-review has **no retry logic** for failed API calls. There is no exponential backoff, no configurable retry count, and no distinction between transient and permanent errors. The only "retry" is the single re-attempt on HTTP 401 with credentials. If any other request fails, the error is immediately propagated to the user and the process exits.

### Timeout Handling

git-review does **not** set timeouts on HTTP requests or SSH processes. The `requests.get()` calls do not pass a `timeout` parameter, so they default to the `requests` library's behavior of waiting indefinitely. SSH processes similarly have no timeout configured, so a hung connection will block the process indefinitely.

### Connection Verification

git-review has a connection verification mechanism in `add_remote()` that tests whether the remote URL works before saving it. It uses `git push --dry-run` as a connectivity test:

```python
def test_remote_url(remote_url):
    status, description = run_command_status("git", "push", "--dry-run",
                                             remote_url, "--all")
    if status != 128:
        return True
    else:
        return False
```

If the test fails (exit code 128 specifically), git-review prompts the user to enter their Gerrit username and tries again. If the second attempt also fails, a `GerritConnectionException` (EXIT_CODE 40) is raised.

### Error Code Summary

The API-related exit codes in git-review's exception hierarchy:

| Exit Code | Exception | Meaning |
|-----------|-----------|---------|
| 1 | `GitReviewException` | Generic error |
| 2 | `CannotInstallHook` | Failed to fetch/install commit-msg hook |
| 32 | `CannotQueryOpenChangesets` | SSH/HTTP query for open changes failed |
| 33 | `CannotParseOpenChangesets` | JSON parsing of change list failed |
| 34 | `CannotQueryPatchSet` | Query for a specific patchset failed |
| 35 | `ReviewInformationNotFound` | Could not extract review info from response |
| 36 | `ReviewNotFound` | Specified change number does not exist |
| 37 | `PatchSetGitFetchFailed` | `git fetch` of a patchset ref failed |
| 38 | `PatchSetNotFound` | Specified patchset number does not exist |
| 40 | `GerritConnectionException` | Cannot connect to Gerrit server |
| 128 | `GitConfigException` | Git config retrieval failed |
| 255 | (synthetic) | Network/connection error in HTTP request |

## grt Divergences

The following areas represent concrete differences between git-review's API interaction approach and how grt will handle the same concerns:

**reqwest async HTTP vs. requests synchronous HTTP.** git-review uses Python's `requests` library, which is synchronous -- each HTTP call blocks the thread until the response arrives. grt will use `reqwest` with tokio's async runtime, allowing HTTP requests to be non-blocking. This enables concurrent API calls (e.g., querying multiple changes in parallel), progressive UI updates during long requests, and cancellation support. The `reqwest` client also provides built-in connection pooling, configurable timeouts, and async streaming, all of which git-review lacks.

**REST-only (no SSH) initially.** git-review's dual-protocol approach -- choosing between SSH and HTTP based on the remote URL -- adds complexity and requires maintaining two parallel implementations for every API operation. grt will initially support only the REST API over HTTP/HTTPS. The SSH protocol adds no API capabilities that the REST API lacks (the REST API is strictly more capable), and eliminating the SSH code path simplifies the client significantly. Users with SSH-only Gerrit setups will need HTTP access configured. The `git push` operation itself may still use SSH transport (since that goes through git, not grt's API client), but grt's own Gerrit queries will exclusively use REST.

**Structured error types vs. string parsing and synthetic exit codes.** git-review maps HTTP status codes to system exit codes via arithmetic (`(code - 301) % 255 + 1`) and wraps all errors in a generic `CommandFailed` exception that stores the raw output string. grt will use Rust's `Result<T, E>` with a typed error enum that distinguishes between network errors, authentication failures, HTTP status errors, JSON parsing errors, and Gerrit-specific API errors. This enables the TUI to present different error states differently (e.g., prompting for credentials on auth failure, showing retry options on transient network errors) rather than treating all failures as fatal string messages.

**Proper timeout and retry support.** git-review has no timeouts and no retry logic. grt will configure `reqwest` with connect and request timeouts, and implement retry with exponential backoff for transient errors (5xx responses, connection resets). The retry policy will be configurable via grt's settings.

**XSSI prefix handling.** git-review hardcodes stripping 4 characters (`request.text[4:]`) to remove Gerrit's XSSI prefix (`)]}'`). This is fragile -- the prefix length could vary across Gerrit versions. grt will look for the prefix pattern and strip it dynamically, handling the case where the prefix is absent (e.g., if a future Gerrit version changes the format or the response comes from a proxy).

**No response normalization.** git-review normalizes the HTTP REST response to match the SSH output format, which means the HTTP path carries an artificial translation cost and is constrained to only the fields that the SSH format provides. grt will work directly with the REST API's native JSON structure, which is richer (includes labels, messages, reviewer lists, file diffs, and other fields that the SSH query format does not expose). This eliminates the normalization code and opens the door to richer data without additional API calls.

**Authentication model.** git-review's auth strategy (unauthenticated request, then retry on 401 with git credentials) is simple but limited. grt will support a broader authentication model: HTTP Basic via configured credentials (from git credential helpers, consistent with git-review's approach), HTTP Bearer tokens, and potentially cookie-based auth for corporate SSO environments. The `reqwest` client's cookie jar support will enable cookie persistence across requests without additional code. Authentication configuration will be centralized in grt's config rather than discovered at runtime through trial-and-error 401 responses.

**No git push --dry-run for connectivity testing.** git-review tests Gerrit connectivity by running `git push --dry-run`, which is heavyweight and requires push permissions. grt will verify connectivity by calling a lightweight REST endpoint (such as `GET /config/server/version` or `GET /accounts/self`) that confirms both network reachability and authentication in a single request without requiring push access.
