# Configuration

grt uses a layered configuration system. Values from later layers override earlier ones.

## Layer Overview

Precedence (lowest to highest):

```
defaults  <  .gitreview  <  grt config  <  git config  <  CLI flags
```

Each layer overrides only the keys it sets; unspecified keys retain values from earlier layers.

## .gitreview

The `.gitreview` file lives at the repository root and uses INI format. It is compatible with git-review.

### Format

- Section: `[gerrit]` (case-insensitive)
- Key-value pairs: `key=value` or `key: value`
- Comments: lines starting with `#` or `;`
- Keys are case-insensitive

### Required vs optional

| Key | Required | Default | Description |
|-----|----------|---------|-------------|
| `host` | Yes | — | Gerrit server hostname |
| `project` | Yes | — | Gerrit project name (`.git` suffix is stripped automatically) |
| `port` | No | — | SSH port (for git remote URLs; REST API uses standard ports) |
| `defaultbranch` | No | `master` | Default target branch for push |
| `defaultremote` | No | `gerrit` | Git remote name |
| `scheme` | No | `https` | URL scheme: `http`, `https`, or `ssh` |
| `defaultrebase` | No | `true` | Rebase before push by default |
| `track` | No | `false` | Use upstream tracking branch as target by default |
| `notopic` | No | `false` | Do not set a topic by default |
| `usepushurl` | No | `false` | Use push URL for remote operations |

### Example

```ini
[gerrit]
host=review.example.com
port=29418
project=my/project.git
defaultbranch=main
defaultremote=gerrit
scheme=https
defaultrebase=1
```

## grt Config

User-level configuration in `~/.config/grt/config.toml` (Linux). On macOS: `~/Library/Application Support/grt/config.toml`. On Windows: `%APPDATA%\grt\config.toml`.

### Format

TOML with a `[gerrit]` table:

```toml
[gerrit]
host = "review.example.com"
port = 8443
project = "my/project"
branch = "main"
remote = "gerrit"
scheme = "https"
```

**Note:** In grt config, `port` maps to the HTTP port used for the REST API. In `.gitreview`, `port` is the SSH port.

## Git Config

Git config keys use the `gitreview.` prefix:

| Key | Description |
|-----|-------------|
| `gitreview.host` | Gerrit hostname |
| `gitreview.hostname` | Alias for host |
| `gitreview.port` | SSH port |
| `gitreview.project` | Project name |
| `gitreview.branch` | Default branch |
| `gitreview.remote` | Remote name |
| `gitreview.username` | HTTP username (for REST API) |

For HTTPS, `http.sslVerify` controls TLS verification (default: true).

## Credentials

Credentials are stored in `~/.config/grt/credentials.toml` (Linux). The file must have mode `0600` (owner read/write only).

### Format

```toml
[[server]]
name = "review.example.com"
username = "alice"
password = "secret-http-password"

[[server]]
name = "review.other.org"
username = "bob"
password = "bearer-token"
auth_type = "bearer"
```

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `name` | Yes | | Gerrit hostname (matched against config) |
| `username` | Yes | | HTTP username |
| `password` | Yes | | HTTP password or bearer token |
| `auth_type` | No | `"basic"` | `"basic"` or `"bearer"` |

When `auth_type` is `"bearer"`, the `password` field is sent as `Authorization: Bearer <token>`.

### Git Credential Helper Fallback

If no matching entry exists in `credentials.toml`, grt falls back to `git credential fill`. After successful auth, `git credential approve` is called; after failure, `git credential reject` is called. Bearer tokens are not supported via the git credential helper.

## URL Rewrites

grt respects git's URL rewrite rules for remote resolution:

- **`url.<base>.insteadOf`** — Rewrites both fetch and push URLs when the URL starts with the given prefix
- **`url.<base>.pushInsteadOf`** — Rewrites only push URLs

Example:

```ini
[url "ssh://git@review.example.com:29418/"]
    insteadOf = https://review.example.com/
```

When resolving the remote URL, grt applies these rewrites using longest-match semantics. For push operations, `pushInsteadOf` takes precedence over `insteadOf` when both match.

## SSH vs HTTP

grt picks the protocol from the resolved remote URL:

- **HTTP/HTTPS** — Uses the Gerrit REST API for change queries, comments, and authentication. Credentials come from `credentials.toml` or the git credential helper.
- **SSH** — Uses `ssh -p <port> <host> gerrit query` for change metadata. No HTTP credentials are needed; SSH key authentication is used.

The remote URL is resolved from `remote.<name>.pushurl` (if set) or `remote.<name>.url`, with `insteadOf` and `pushInsteadOf` applied. The scheme of the resulting URL determines whether grt uses REST or SSH for Gerrit operations.
