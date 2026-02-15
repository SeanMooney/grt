// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

//! Protocol-aware Gerrit query layer.
//!
//! Dispatches change queries to HTTP/REST or SSH based on the remote URL,
//! matching git-review's dual-protocol behavior.

use std::path::Path;

use anyhow::{Context, Result};

use crate::config::{alias_url, populate_rewrites};
use crate::gerrit::{ChangeInfo, GerritClient};
use crate::list;
use crate::subprocess;

/// Resolve the effective remote URL for push operations.
///
/// Matches git-review's `get_remote_url`: prefers `remote.<name>.pushurl`,
/// applies `insteadOf` to pushurl; falls back to `remote.<name>.url` with
/// `pushInsteadOf` then `insteadOf`.
pub fn resolve_remote_url(
    remote: &str,
    work_dir: &Path,
    fallback_url: Option<&str>,
) -> Result<Option<String>> {
    let rewrites = subprocess::git_config_list(work_dir)
        .map(|out| populate_rewrites(&out))
        .unwrap_or_default();

    let pushurl = subprocess::git_config_get(&format!("remote.{remote}.pushurl"), work_dir)?;
    let url_raw = subprocess::git_config_get(&format!("remote.{remote}.url"), work_dir)?;

    let url = match (&pushurl, &url_raw) {
        (Some(u), _) => {
            // pushurl: apply insteadOf only (not pushInsteadOf)
            alias_url(u, &rewrites, false)
        }
        (None, Some(u)) => {
            // url: apply pushInsteadOf then insteadOf
            alias_url(u, &rewrites, true)
        }
        (None, None) => fallback_url.map(String::from).unwrap_or_default(),
    };

    if url.is_empty() {
        return Ok(None);
    }
    Ok(Some(url))
}

/// Return true if the URL uses HTTP or HTTPS (REST API path).
#[inline]
pub fn is_http_remote(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_http_remote_https() {
        assert!(is_http_remote("https://gerrit.example.com/project"));
    }

    #[test]
    fn is_http_remote_http() {
        assert!(is_http_remote("http://gerrit.example.com/project"));
    }

    #[test]
    fn is_http_remote_ssh() {
        assert!(!is_http_remote(
            "ssh://user@gerrit.example.com:29418/project"
        ));
    }

    #[test]
    fn is_http_remote_scp() {
        assert!(!is_http_remote("git@gerrit.example.com:project/repo"));
    }

    #[test]
    fn resolve_remote_url_fallback_when_no_remote() {
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(work_dir)
            .output()
            .unwrap();
        let url =
            resolve_remote_url("origin", work_dir, Some("https://fallback.example.com/p")).unwrap();
        assert_eq!(url.as_deref(), Some("https://fallback.example.com/p"));
    }

    #[test]
    fn resolve_remote_url_with_pushurl() {
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "remote.origin.url", "https://fetch.example.com/p"])
            .current_dir(work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "config",
                "remote.origin.pushurl",
                "ssh://user@push.example.com:29418/p",
            ])
            .current_dir(work_dir)
            .output()
            .unwrap();
        let url = resolve_remote_url("origin", work_dir, None).unwrap();
        assert_eq!(url.as_deref(), Some("ssh://user@push.example.com:29418/p"));
    }
}

/// Query open changes, dispatching to HTTP or SSH based on remote URL.
pub async fn query_changes(
    remote_url: &str,
    project: &str,
    branch: Option<&str>,
    gerrit: &GerritClient,
    work_dir: &Path,
) -> Result<Vec<ChangeInfo>> {
    if is_http_remote(remote_url) {
        let query = list::build_list_query(project, branch);
        gerrit.query_changes(&query).await
    } else {
        ssh::query_changes_over_ssh(remote_url, project, branch, work_dir).await
    }
}

/// Get change detail with all revisions (for download/cherry-pick/compare).
pub async fn get_change_all_revisions(
    remote_url: &str,
    change_id: &str,
    gerrit: &GerritClient,
    work_dir: &Path,
) -> Result<ChangeInfo> {
    if is_http_remote(remote_url) {
        gerrit.get_change_all_revisions(change_id).await
    } else {
        ssh::get_change_all_revisions_ssh(remote_url, change_id, work_dir).await
    }
}

/// SSH-based Gerrit query backend.
mod ssh {
    use super::*;
    use crate::gerrit::AccountInfo;
    use serde::Deserialize;
    use std::collections::HashMap;
    use std::process::Command;

    /// SSH query output uses `number`, `currentPatchSet`, `patchSets` (not REST's _number/revisions).
    /// Gerrit SSH uses createdOn/lastUpdated (not created/updated) and may use id for Change-Id.
    #[derive(Debug, Deserialize)]
    struct SshChangeRaw {
        #[serde(alias = "_number")]
        number: Option<i64>,
        id: Option<String>,
        project: Option<String>,
        branch: Option<String>,
        change_id: Option<String>,
        subject: Option<String>,
        status: Option<String>,
        topic: Option<String>,
        #[serde(
            default,
            alias = "createdOn",
            deserialize_with = "deserialize_optional_string_flexible"
        )]
        created: Option<String>,
        #[serde(
            default,
            alias = "lastUpdated",
            deserialize_with = "deserialize_optional_string_flexible"
        )]
        updated: Option<String>,
        owner: Option<AccountInfo>,
        #[serde(rename = "currentPatchSet")]
        current_patch_set: Option<SshPatchSet>,
        #[serde(rename = "patchSets")]
        patch_sets: Option<Vec<SshPatchSet>>,
    }

    #[derive(Debug, Deserialize)]
    struct SshPatchSet {
        #[serde(deserialize_with = "deserialize_optional_i32_flexible")]
        number: Option<i32>,
        #[serde(rename = "ref")]
        git_ref: Option<String>,
        revision: Option<String>,
    }

    /// Accept patch set number as integer or string (some Gerrit configs emit string).
    fn deserialize_optional_i32_flexible<'de, D>(d: D) -> Result<Option<i32>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;
        let v: Option<serde_json::Value> = Option::deserialize(d)?;
        match v {
            None => Ok(None),
            Some(serde_json::Value::Number(n)) => n
                .as_i64()
                .and_then(|i| i32::try_from(i).ok())
                .map(Some)
                .ok_or_else(|| D::Error::custom("patch set number out of range")),
            Some(serde_json::Value::String(s)) => s
                .parse::<i32>()
                .map(Some)
                .map_err(|_| D::Error::custom("patch set number not a valid integer")),
            _ => Ok(None),
        }
    }

    /// Accept timestamp as string or number (Gerrit SSH uses epoch seconds for createdOn/lastUpdated).
    fn deserialize_optional_string_flexible<'de, D>(d: D) -> Result<Option<String>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v: Option<serde_json::Value> = Option::deserialize(d)?;
        match v {
            None => Ok(None),
            Some(serde_json::Value::String(s)) => Ok(Some(s)),
            Some(serde_json::Value::Number(n)) => Ok(n.as_i64().map(|i| i.to_string())),
            _ => Ok(None),
        }
    }

    fn ssh_change_to_change_info(raw: SshChangeRaw) -> ChangeInfo {
        let (revisions, current_revision) = if let Some(ref patch_sets) = raw.patch_sets {
            let mut revs = HashMap::new();
            let mut current = None;
            let mut highest_num: Option<i32> = None;
            for ps in patch_sets {
                if let (Some(rev), Some(num), Some(r)) =
                    (ps.revision.clone(), ps.number, ps.git_ref.clone())
                {
                    revs.insert(
                        rev.clone(),
                        crate::gerrit::RevisionInfo {
                            number: Some(num),
                            git_ref: Some(r),
                            commit: None,
                        },
                    );
                    if raw
                        .current_patch_set
                        .as_ref()
                        .map(|cps| cps.number == Some(num))
                        .unwrap_or(false)
                    {
                        current = Some(rev);
                    }
                    if highest_num.is_none_or(|h| num > h) {
                        highest_num = Some(num);
                    }
                }
            }
            if current.is_none() {
                current = raw
                    .current_patch_set
                    .as_ref()
                    .and_then(|cps| cps.revision.clone());
            }
            if current.is_none() {
                current = highest_num.and_then(|hn| {
                    patch_sets
                        .iter()
                        .find(|ps| ps.number == Some(hn))
                        .and_then(|ps| ps.revision.clone())
                });
            }
            (Some(revs), current)
        } else if let Some(ref cps) = raw.current_patch_set {
            let mut revs = HashMap::new();
            if let (Some(rev), Some(num), Some(r)) =
                (cps.revision.clone(), cps.number, cps.git_ref.clone())
            {
                revs.insert(
                    rev.clone(),
                    crate::gerrit::RevisionInfo {
                        number: Some(num),
                        git_ref: Some(r),
                        commit: None,
                    },
                );
                (Some(revs), Some(rev))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        ChangeInfo {
            id: raw.id.clone(),
            project: raw.project,
            branch: raw.branch,
            change_id: raw.change_id.or(raw.id),
            subject: raw.subject,
            status: raw.status,
            topic: raw.topic,
            created: raw.created,
            updated: raw.updated,
            number: raw.number,
            owner: raw.owner,
            current_revision,
            revisions,
            messages: None,
            insertions: None,
            deletions: None,
        }
    }

    /// Parse SSH/SCP-style URL into (hostname, username, port, project).
    ///
    /// Supports `ssh://[user@]host[:port]/path` and `user@host:path`.
    pub fn parse_gerrit_ssh_params(
        url: &str,
    ) -> Result<(String, Option<String>, Option<u16>, String)> {
        if url.contains("://") {
            parse_ssh_url_format(url)
        } else {
            parse_scp_format(url)
        }
    }

    fn parse_ssh_url_format(url: &str) -> Result<(String, Option<String>, Option<u16>, String)> {
        let rest = url.strip_prefix("ssh://").context("expected ssh:// URL")?;

        let (userhost, path) = rest
            .split_once('/')
            .context("SSH URL has no path component")?;

        let (hostname, username, port) = parse_userhost(userhost)?;
        let project = path
            .trim_start_matches('/')
            .strip_suffix(".git")
            .unwrap_or(path.trim_start_matches('/'))
            .to_string();

        Ok((hostname, username, port, project))
    }

    fn parse_scp_format(url: &str) -> Result<(String, Option<String>, Option<u16>, String)> {
        let (host_part, path) = url
            .split_once(':')
            .context("SCP URL must have host:path form")?;

        if path.starts_with("//") {
            anyhow::bail!("ambiguous SCP URL (path starts with //)");
        }

        let (hostname, username) = if let Some((user, host)) = host_part.rsplit_once('@') {
            (host.to_string(), Some(user.to_string()))
        } else {
            (host_part.to_string(), None)
        };

        let project = path.strip_suffix(".git").unwrap_or(path).to_string();

        Ok((hostname, username, None, project))
    }

    fn parse_userhost(userhost: &str) -> Result<(String, Option<String>, Option<u16>)> {
        let (userhost, port) = if let Some((uh, port_str)) = userhost.rsplit_once(':') {
            let port: u16 = port_str.parse().context("parsing port from URL")?;
            (uh, Some(port))
        } else {
            (userhost, None)
        };

        let (hostname, username) = if let Some((user, host)) = userhost.rsplit_once('@') {
            (host.to_string(), Some(user.to_string()))
        } else {
            (userhost.to_string(), None)
        };

        Ok((hostname, username, port))
    }

    /// Run `ssh ... gerrit query --format=JSON ...` and parse JSON-per-line output.
    pub async fn query_changes_over_ssh(
        remote_url: &str,
        project: &str,
        branch: Option<&str>,
        work_dir: &Path,
    ) -> Result<Vec<ChangeInfo>> {
        let (hostname, username, port, _project_name) = parse_gerrit_ssh_params(remote_url)?;

        let project_clean = project.strip_suffix(".git").unwrap_or(project);
        let mut query = format!("project:{} status:open", project_clean);
        if let Some(b) = branch {
            query.push_str(&format!(" branch:{b}"));
        }

        let output =
            run_gerrit_query_ssh(&hostname, username.as_deref(), port, &query, work_dir).await?;
        parse_ssh_query_output(&output)
    }

    /// Get change with all revisions via SSH.
    pub async fn get_change_all_revisions_ssh(
        remote_url: &str,
        change_id: &str,
        work_dir: &Path,
    ) -> Result<ChangeInfo> {
        let (hostname, username, port, _project) = parse_gerrit_ssh_params(remote_url)?;

        // --current-patch-set ensures currentPatchSet is present; --patch-sets adds all patchSets
        let query = format!("--current-patch-set --patch-sets change:{change_id}");
        let output =
            run_gerrit_query_ssh(&hostname, username.as_deref(), port, &query, work_dir).await?;
        let changes = parse_ssh_query_output(&output)?;

        changes
            .into_iter()
            .next()
            .context("change not found in SSH query output")
    }

    async fn run_gerrit_query_ssh(
        hostname: &str,
        username: Option<&str>,
        port: Option<u16>,
        query: &str,
        work_dir: &Path,
    ) -> Result<String> {
        let userhost = match username {
            Some(u) => format!("{u}@{hostname}"),
            None => hostname.to_string(),
        };

        let port_arg = match port {
            Some(p) => format!("-p{p}"),
            None => "-p29418".to_string(),
        };

        let ssh_bin = std::env::var("GIT_SSH").unwrap_or_else(|_| "ssh".to_string());
        let full_query = format!("--format=JSON {query}");
        let work_dir = work_dir.to_path_buf();

        let output = tokio::task::spawn_blocking(move || {
            Command::new(&ssh_bin)
                .args(["-x", &port_arg, &userhost, "gerrit", "query", &full_query])
                .current_dir(&work_dir)
                .env("LANG", "C")
                .env("LANGUAGE", "C")
                .output()
        })
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking: {e}"))?
        .context("running ssh gerrit query")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("ssh gerrit query failed: {}", stderr.trim());
        }

        String::from_utf8(output.stdout).context("ssh output is not valid UTF-8")
    }

    fn parse_ssh_query_output(output: &str) -> Result<Vec<ChangeInfo>> {
        let mut changes = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if !line.starts_with('{') {
                continue;
            }
            let data: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if data.get("type").is_some() {
                continue;
            }
            let raw: SshChangeRaw =
                serde_json::from_str(line).context("parsing SSH query JSON line")?;
            changes.push(ssh_change_to_change_info(raw));
        }
        Ok(changes)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parse_ssh_url_standard() {
            let (host, user, port, proj) =
                parse_gerrit_ssh_params("ssh://alice@review.example.com:29418/project").unwrap();
            assert_eq!(host, "review.example.com");
            assert_eq!(user.as_deref(), Some("alice"));
            assert_eq!(port, Some(29418));
            assert_eq!(proj, "project");
        }

        #[test]
        fn parse_scp_format() {
            let (host, user, port, proj) =
                parse_gerrit_ssh_params("git@review.example.com:project/repo").unwrap();
            assert_eq!(host, "review.example.com");
            assert_eq!(user.as_deref(), Some("git"));
            assert_eq!(port, None);
            assert_eq!(proj, "project/repo");
        }

        #[test]
        fn parse_strips_dotgit() {
            let (_, _, _, proj) = parse_gerrit_ssh_params("ssh://host/repo.git").unwrap();
            assert_eq!(proj, "repo");
        }

        #[test]
        fn parse_ssh_query_output_skips_stats_lines() {
            // Gerrit SSH query emits JSON-per-line; lines with "type" are stats, not changes
            // SSH output omits _account_id in owner (unlike REST); AccountInfo must tolerate that
            let output = r#"{"type":"stats","rowCount":2,"runTimeMilliseconds":5}
{"id":"abc","project":"p","change_id":"I123","subject":"Fix bug","status":"NEW","owner":{"name":"Alice","email":"alice@example.com"},"currentPatchSet":{"number":1,"ref":"refs/changes/1/1/1","revision":"abc123"}}
{"type":"stats","rowCount":2,"runTimeMilliseconds":5}"#;
            let changes = parse_ssh_query_output(output).unwrap();
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].change_id.as_deref(), Some("I123"));
            assert_eq!(changes[0].subject.as_deref(), Some("Fix bug"));
            assert_eq!(
                changes[0].owner.as_ref().and_then(|o| o.name.as_deref()),
                Some("Alice")
            );
        }

        #[test]
        fn parse_ssh_query_output_createdon_lastupdated() {
            // Gerrit SSH uses createdOn/lastUpdated (epoch seconds) or string; we accept both
            let output = r#"{"id":"I123","project":"p","subject":"Fix","status":"NEW","createdOn":1706788800,"lastUpdated":1707580800,"currentPatchSet":{"number":1,"ref":"refs/changes/1/1/1","revision":"abc123"}}"#;
            let changes = parse_ssh_query_output(output).unwrap();
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].created.as_deref(), Some("1706788800"));
            assert_eq!(changes[0].updated.as_deref(), Some("1707580800"));
        }

        #[test]
        fn parse_ssh_query_output_patchset_number_as_string() {
            // Some Gerrit configs emit patch set number as string
            let output = r#"{"id":"I123","project":"p","subject":"Fix","status":"NEW","currentPatchSet":{"number":"1","ref":"refs/changes/1/1/1","revision":"abc123"},"patchSets":[{"number":"1","ref":"refs/changes/1/1/1","revision":"abc123"}]}"#;
            let changes = parse_ssh_query_output(output).unwrap();
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].current_revision.as_deref(), Some("abc123"));
            assert_eq!(
                changes[0]
                    .revisions
                    .as_ref()
                    .and_then(|r| r.get("abc123"))
                    .and_then(|rev| rev.number),
                Some(1)
            );
        }
    }
}
