# grt Configuration System

**Related ref-specs:** `../ref-specs/gertty-config-and-ui.md`
**Source code:** `crates/grt/src/config.rs`, `crates/grt/src/app.rs`
**Status:** Draft

## Overview

grt uses a four-tier layered configuration system:

1. `.gitreview` file (INI format, git-review compatible)
2. `~/.config/grt/config.toml` (grt native TOML config)
3. Git config (`gitreview.*` keys)
4. CLI flags (highest precedence)

Additionally, credentials are stored separately in `~/.config/grt/credentials.toml` with strict file permission enforcement, and git's credential helper system serves as a fallback.

The layering is applied in `load_config()`, which produces a `GerritConfig` struct containing all resolved values. Each subsequent layer overrides values from the previous one, with CLI flags having the highest precedence.

## Config Sources

### `.gitreview` (INI Format)

The `.gitreview` file is an INI file at the repository root, compatible with git-review. It uses a `[gerrit]` section with key=value pairs.

```ini
[gerrit]
host=review.example.com
port=29418
project=my/project.git
defaultbranch=develop
defaultremote=gerrit
scheme=https
```

**Supported keys:**

| Key | Maps to | Description |
|-----|---------|-------------|
| `host` | `config.host` | Gerrit server hostname |
| `port` | `config.ssh_port` | SSH port (for git remote URLs, **not** REST API) |
| `project` | `config.project` | Gerrit project name |
| `defaultbranch` | `config.branch` | Default target branch for push |
| `defaultremote` | `config.remote` | Git remote name |
| `scheme` | `config.scheme` | URL scheme (http/https) |

**Parser implementation:** grt includes a hand-written INI parser (`parse_gitreview`) rather than using a crate. It scans for a `[gerrit]` section (case-insensitive), then collects key=value pairs, trimming whitespace around both keys and values. Lines starting with `#` or `;` are treated as comments. Missing the `[gerrit]` section is an error.

**Port semantics:** The `.gitreview` `port` field is treated as the SSH port (`config.ssh_port`), not the HTTP port. This matches git-review's convention where the port is used for SSH remote URL construction (e.g., `ssh://user@host:29418/project`). The REST API URL uses the standard port for the scheme (443 for HTTPS, 80 for HTTP) unless explicitly overridden via grt config or CLI.

### `.git` Suffix Stripping

Many `.gitreview` files include a `.git` suffix on the project name (e.g., `project=openstack/watcher.git`). However, the Gerrit REST API expects bare project names without the suffix. grt automatically strips the `.git` suffix from project names at all four config loading layers:

```rust
fn strip_git_suffix(project: &str) -> String {
    project.strip_suffix(".git").unwrap_or(project).to_string()
}
```

This is applied in:
1. `.gitreview` parsing
2. TOML config loading
3. Git config loading
4. CLI override application

### `~/.config/grt/config.toml` (Native TOML Config)

grt's native configuration file, located at the platform-specific config directory (`dirs::config_dir()`). Uses a `[gerrit]` table:

```toml
[gerrit]
host = "review.example.com"
port = 8443
project = "my/project"
branch = "main"
remote = "gerrit"
scheme = "https"
```

**Port semantics:** Unlike `.gitreview`, the `port` in grt config maps to `config.http_port` and is used in REST API URL construction.

### Git Config (`gitreview.*`)

Git config values are read via gix's config snapshot. Keys use the `gitreview.` prefix, matching git-review's convention:

| Git Config Key | Maps to | Description |
|----------------|---------|-------------|
| `gitreview.host` | `config.host` | Gerrit hostname |
| `gitreview.port` | `config.ssh_port` | SSH port |
| `gitreview.project` | `config.project` | Project name |
| `gitreview.branch` | `config.branch` | Default branch |
| `gitreview.remote` | `config.remote` | Remote name |

### CLI Flags

CLI flags have the highest precedence. They are passed through the `CliOverrides` struct.

## Config Precedence

Values are resolved in this order, with later sources overriding earlier ones:

```
Defaults → .gitreview → config.toml → git config → CLI flags
```

**Defaults** (`GerritConfig::default()`):

| Field | Default |
|-------|---------|
| `host` | `""` (empty -- must be configured) |
| `ssh_port` | `None` |
| `http_port` | `None` |
| `project` | `""` (empty -- must be configured) |
| `branch` | `"main"` |
| `remote` | `"gerrit"` |
| `scheme` | `"https"` |

After loading, if `config.host` is still empty, `App::new()` returns an error directing the user to create a `.gitreview` file or set `gitreview.host` in git config.

## Credential Management

### `~/.config/grt/credentials.toml`

Credentials are stored separately from general configuration in a dedicated file with strict permissions.

```toml
[[server]]
name = "review.opendev.org"
username = "alice"
password = "secret-http-password"

[[server]]
name = "review.other.org"
username = "bob"
password = "another-token"
auth_type = "bearer"
```

The `name` field is matched against `config.host` to find the appropriate credentials.

**Fields:**

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `name` | Yes | | Gerrit server hostname to match |
| `username` | Yes | | HTTP username |
| `password` | Yes | | HTTP password or bearer token |
| `auth_type` | No | `"basic"` | Authentication type: `"basic"` or `"bearer"` |

**Auth type handling:** When `auth_type` is `"bearer"`, the `password` field is treated as a bearer token and sent via `Authorization: Bearer <token>` header. The default `auth_type` is `"basic"` for backwards compatibility.

**File permission enforcement (Unix):** The file must have mode `0600` (owner read/write only). If permissions are looser, `load_credentials` returns an error with an actionable message:

```
credentials.toml has permissions 0644, expected 0600.
Fix with: chmod 600 /home/user/.config/grt/credentials.toml
```

### LoadedCredentials Struct

The `load_credentials()` function returns `Option<LoadedCredentials>`:

```rust
pub struct LoadedCredentials {
    pub username: String,
    pub password: String,
    pub auth_type: AuthType,
}
```

This struct propagates the auth type from `credentials.toml` through to `app.rs`, which passes it to `GerritClient::set_credentials()`.

### Git Credential Helper Fallback

If no matching entry is found in `credentials.toml` (or the file doesn't exist), grt falls back to git's credential helper system:

1. `git credential fill` is invoked via subprocess with the Gerrit URL's protocol and host
2. The helper returns username/password on stdout
3. After successful authentication, `git credential approve` is called
4. After failed authentication, `git credential reject` is called

Credentials from the git helper always use `AuthType::Basic` (no bearer token support via git credential).

The credential source is tracked internally (`CredentialSource::File` or `CredentialSource::GitHelper`) so that `approve`/`reject` calls are only made for git-helper-sourced credentials.

### TLS Enforcement

`App::authenticate()` refuses to send credentials over plain HTTP:

```rust
if self.config.scheme != "https" && !self.insecure {
    anyhow::bail!(
        "refusing to send credentials over plain HTTP (scheme: {}). \
         Use --insecure to override, or switch to HTTPS",
        self.config.scheme,
    );
}
```

The `--insecure` global flag overrides this check.

## Platform Paths

| Platform | Config directory | Credentials path |
|----------|-----------------|------------------|
| Linux | `~/.config` (XDG_CONFIG_HOME) | `~/.config/grt/credentials.toml` |
| macOS | `~/Library/Application Support` | `~/Library/Application Support/grt/credentials.toml` |
| Windows | `%APPDATA%` | `%APPDATA%\grt\credentials.toml` |

## Validation

### Host Requirement

After config loading, `App::new()` validates that a Gerrit host is configured.

### URL Construction

`GerritConfig::gerrit_base_url()` constructs the REST API base URL, using `http_port` if explicitly set, otherwise defaulting to the standard port for the scheme.

The SSH port (`ssh_port`) is intentionally not used for REST API URLs. This is a deliberate design choice -- `.gitreview` files commonly set `port=29418` for SSH, which would produce an invalid REST URL if used for HTTP.

### INI Parsing Errors

The `.gitreview` parser returns specific errors:

- Missing `[gerrit]` section: `"missing [gerrit] section in .gitreview"`
- Invalid port: `"parsing port in .gitreview"` (via `.context()`)

## Multi-Server Support

Not yet implemented. The `credentials.toml` file already supports multiple `[[server]]` entries, but the config system resolves to a single `GerritConfig` based on the current repository's `.gitreview` file.

## Divergences from Ref-Specs

### vs. gertty (`gertty-config-and-ui.md`)

- **TOML + INI hybrid vs. YAML**: gertty uses a single YAML file with voluptuous schema validation. grt uses TOML for native config and INI for `.gitreview` compatibility.
- **Layered config**: gertty has a two-tier model (defaults + YAML). grt has four tiers.
- **Separate credentials**: gertty stores passwords inline in its config file. grt separates credentials into a dedicated file with stricter permission enforcement.
- **Auth type support**: gertty supports `basic`, `digest`, and `form` auth types per server. grt supports `basic` and `bearer`.
- **No config-file-level validation**: gertty validates its YAML against a voluptuous schema. grt uses manual field extraction.
- **No interactive password prompt**: gertty prompts for a password via `getpass` if not in config. grt falls back to `git credential fill` instead.
- **`.git` suffix stripping**: gertty does not strip `.git` suffixes. grt automatically strips them for Gerrit API compatibility.

### vs. git-review (`git-review-workflow.md`)

- **Compatible `.gitreview` format**: grt reads the same `.gitreview` files as git-review, making migration seamless.
- **`.git` suffix handling**: git-review passes the `.git` suffix through. grt strips it, since the Gerrit REST API expects bare project names.
- **SSH vs HTTP port distinction**: git-review uses the `.gitreview` port for both SSH remote URLs and (confusingly) sometimes for HTTP. grt explicitly separates `ssh_port` and `http_port`.
- **Bearer auth**: git-review does not support bearer token authentication. grt supports it via `auth_type = "bearer"` in credentials.toml.
- **Config loading at startup**: git-review reads git config via subprocess on every key access. grt loads all config once at startup into a structured `GerritConfig` type via gix's config snapshot.
