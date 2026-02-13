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
project=my/project
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

**Supported keys:**

| Key | Maps to | Description |
|-----|---------|-------------|
| `host` | `config.host` | Gerrit server hostname |
| `port` | `config.http_port` | HTTP port for REST API |
| `project` | `config.project` | Gerrit project name |
| `branch` | `config.branch` | Default target branch |
| `remote` | `config.remote` | Git remote name |
| `scheme` | `config.scheme` | URL scheme |

**Port semantics:** Unlike `.gitreview`, the `port` in grt config maps to `config.http_port` and is used in REST API URL construction. This avoids the confusion of using SSH ports for HTTP connections.

The TOML config is parsed using the `toml` crate's `Table` type, with manual field extraction. This approach allows the file to contain additional sections without causing parse errors.

### Git Config (`gitreview.*`)

Git config values are read via gix's config snapshot. Keys use the `gitreview.` prefix, matching git-review's convention:

| Git Config Key | Maps to | Description |
|----------------|---------|-------------|
| `gitreview.host` | `config.host` | Gerrit hostname |
| `gitreview.port` | `config.ssh_port` | SSH port |
| `gitreview.project` | `config.project` | Project name |
| `gitreview.branch` | `config.branch` | Default branch |
| `gitreview.remote` | `config.remote` | Remote name |

Git config values can be set at any level (system, global, local) and follow git's standard precedence rules.

### CLI Flags

CLI flags have the highest precedence and override all other sources. They are passed through the `CliOverrides` struct:

```rust
pub struct CliOverrides {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub remote: Option<String>,
    pub scheme: Option<String>,
    pub insecure: bool,
}
```

In the MVP, only `--remote`, `--branch` (positional on push), and `--insecure` are exposed as CLI flags. The `CliOverrides` struct supports additional overrides for future use.

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
```

The `name` field is matched against `config.host` to find the appropriate credentials.

**File permission enforcement (Unix):** The file must have mode `0600` (owner read/write only). If permissions are looser, `load_credentials` returns an error with an actionable message:

```
credentials.toml has permissions 0644, expected 0600.
Fix with: chmod 600 /home/user/.config/grt/credentials.toml
```

This follows the pattern used by gertty, which enforces the same permission on its YAML config when passwords are present.

### Git Credential Helper Fallback

If no matching entry is found in `credentials.toml` (or the file doesn't exist), grt falls back to git's credential helper system:

1. `git credential fill` is invoked via subprocess with the Gerrit URL's protocol and host
2. The helper returns username/password on stdout
3. After successful authentication, `git credential approve` is called to let the helper cache the credentials
4. After failed authentication, `git credential reject` is called to invalidate cached credentials

This integrates with any credential helper the user has configured (e.g., `credential.helper=store`, `credential.helper=cache`, OS keychain helpers).

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

Platform-specific config directory resolution uses the `dirs` crate:

| Platform | Config directory | Credentials path |
|----------|-----------------|------------------|
| Linux | `~/.config` (XDG_CONFIG_HOME) | `~/.config/grt/credentials.toml` |
| macOS | `~/Library/Application Support` | `~/Library/Application Support/grt/credentials.toml` |
| Windows | `%APPDATA%` | `%APPDATA%\grt\credentials.toml` |

The `dirs::config_dir()` function handles platform detection and XDG compliance on Linux.

## Validation

### Host Requirement

After config loading, `App::new()` validates that a Gerrit host is configured:

```rust
if config.host.is_empty() {
    anyhow::bail!(
        "no Gerrit host configured. Create a .gitreview file or \
         set gitreview.host in git config"
    );
}
```

### URL Construction

`GerritConfig::gerrit_base_url()` constructs the REST API base URL, using `http_port` if explicitly set, otherwise defaulting to the standard port for the scheme:

```rust
pub fn gerrit_base_url(&self) -> Result<Url> {
    let url_str = match self.http_port {
        Some(port) => format!("{}://{}:{}", self.scheme, self.host, port),
        None => format!("{}://{}", self.scheme, self.host),
    };
    Url::parse(&url_str).context("constructing Gerrit base URL")
}
```

The SSH port (`ssh_port`) is intentionally not used for REST API URLs. This is a deliberate design choice -- `.gitreview` files commonly set `port=29418` for SSH, which would produce an invalid REST URL if used for HTTP.

### INI Parsing Errors

The `.gitreview` parser returns specific errors:

- Missing `[gerrit]` section: `"missing [gerrit] section in .gitreview"`
- Invalid port: `"parsing port in .gitreview"` (via `.context()`)

## Multi-Server Support

Not yet implemented. The `credentials.toml` file already supports multiple `[[server]]` entries, but the config system resolves to a single `GerritConfig` based on the current repository's `.gitreview` file.

Future work will add a `[servers]` table in `config.toml` for defining multiple named servers with per-server settings (HTTP port, scheme, auth type).

## Divergences from Ref-Specs

### vs. gertty (`gertty-config-and-ui.md`)

- **TOML + INI hybrid vs. YAML**: gertty uses a single YAML file with voluptuous schema validation. grt uses TOML for native config and INI for `.gitreview` compatibility.
- **Layered config**: gertty has a two-tier model (defaults + YAML). grt has four tiers (defaults + .gitreview + config.toml + git config + CLI flags).
- **Separate credentials**: gertty stores passwords inline in its config file. grt separates credentials into a dedicated file with stricter permission enforcement.
- **No schema validation**: gertty validates its YAML against a voluptuous schema at load time. grt uses manual field extraction with serde for TOML and a hand-written parser for INI.
- **No interactive password prompt**: gertty prompts for a password via `getpass` if not in config. grt falls back to `git credential fill` instead.
- **No config-file-level auth-type**: gertty supports `basic`, `digest`, and `form` auth types per server. grt uses HTTP Basic exclusively.

### vs. git-review (`git-review-workflow.md`)

- **Compatible `.gitreview` format**: grt reads the same `.gitreview` files as git-review, making migration seamless.
- **No `defaultrebase` support**: git-review reads `defaultrebase` from `.gitreview`. grt does not implement rebase configuration.
- **SSH vs HTTP port distinction**: git-review uses the `.gitreview` port for both SSH remote URLs and (confusingly) sometimes for HTTP. grt explicitly separates `ssh_port` and `http_port`.
- **No `gitreview.username`**: git-review reads a username from git config for SSH remote URL construction. grt does not need this since it uses REST (not SSH) and sources credentials from `credentials.toml` or the git credential helper.
- **Config loading at startup**: git-review reads git config via subprocess on every key access. grt loads all config once at startup into a structured `GerritConfig` type via gix's config snapshot.
