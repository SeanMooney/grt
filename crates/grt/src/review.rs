// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use anyhow::{Context, Result};
use clap::Args;
use tracing::debug;

use crate::app::App;
use crate::gerrit::{ChangeInfo, RevisionInfo};
use crate::list;
use crate::subprocess;

/// ReviewArgs mirrors git-review's exact flag set.
///
/// Shared between the `grt review` subcommand and the busybox `git-review`
/// argv\[0\] mode. When no mode flag is set, the default action is push.
#[derive(Args, Debug)]
pub struct ReviewArgs {
    /// Target branch (defaults to config or "main")
    pub branch: Option<String>,

    // === Mode flags (mutually exclusive group) ===
    /// Download a change from Gerrit
    #[arg(short = 'd', long = "download", value_name = "CHANGE", group = "mode")]
    pub download: Option<String>,

    /// Cherry-pick a change onto the current branch
    #[arg(short = 'x', long, value_name = "CHANGE", group = "mode")]
    pub cherrypick: Option<String>,

    /// Cherry-pick with "(cherry picked from ...)" indication
    #[arg(short = 'X', long, value_name = "CHANGE", group = "mode")]
    pub cherrypickindicate: Option<String>,

    /// Cherry-pick without committing (apply to working directory only)
    #[arg(short = 'N', long, value_name = "CHANGE", group = "mode")]
    pub cherrypickonly: Option<String>,

    /// Compare patchsets of a change
    #[arg(short = 'm', long, value_name = "CHANGE,PS[-PS]", group = "mode")]
    pub compare: Option<String>,

    /// List open changes (-l brief, -ll verbose)
    #[arg(short = 'l', long, action = clap::ArgAction::Count, group = "mode")]
    pub list: u8,

    /// Set up the current repository for Gerrit
    #[arg(short = 's', long, group = "mode")]
    pub setup: bool,

    // === Topic (mutually exclusive) ===
    /// Set the topic for the push
    #[arg(short = 't', long, value_name = "TOPIC", conflicts_with = "no_topic")]
    pub topic: Option<String>,

    /// Do not set a topic
    #[arg(short = 'T', long)]
    pub no_topic: bool,

    // === Rebase (mutually exclusive) ===
    /// Do not rebase before pushing
    #[arg(short = 'R', long, conflicts_with = "force_rebase")]
    pub no_rebase: bool,

    /// Force rebase before pushing
    #[arg(short = 'F', long)]
    pub force_rebase: bool,

    // === Track (mutually exclusive) ===
    /// Use the upstream tracking branch as the target
    #[arg(long, conflicts_with = "no_track")]
    pub track: bool,

    /// Ignore upstream tracking branch
    #[arg(long)]
    pub no_track: bool,

    // === WIP (mutually exclusive) ===
    /// Mark as work-in-progress
    #[arg(
        short = 'w',
        long = "wip",
        visible_alias = "work-in-progress",
        conflicts_with = "ready"
    )]
    pub wip: bool,

    /// Mark as ready for review
    #[arg(short = 'W', long)]
    pub ready: bool,

    // === Privacy (mutually exclusive) ===
    /// Mark as private
    #[arg(short = 'p', long, conflicts_with = "remove_private")]
    pub private: bool,

    /// Remove the private flag
    #[arg(short = 'P', long)]
    pub remove_private: bool,

    // === Push metadata ===
    /// Add reviewers
    #[arg(long, value_name = "USER", num_args = 1..)]
    pub reviewers: Vec<String>,

    /// Add CC recipients
    #[arg(long, value_name = "USER", num_args = 1..)]
    pub cc: Vec<String>,

    /// Add hashtags
    #[arg(long, value_name = "TAG", num_args = 1..)]
    pub hashtags: Vec<String>,

    /// Notification setting (NONE, OWNER, OWNER_REVIEWERS, ALL)
    #[arg(long, value_name = "LEVEL")]
    pub notify: Option<String>,

    /// Review message
    #[arg(long, value_name = "TEXT")]
    pub message: Option<String>,

    // === Behavior flags ===
    /// Remote to push to
    #[arg(short = 'r', long, value_name = "REMOTE")]
    pub remote: Option<String>,

    /// Show what would be done without doing it
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Generate a new Change-Id (amend HEAD)
    #[arg(short = 'i', long)]
    pub new_changeid: bool,

    /// Skip confirmation prompts
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Run `git remote update` before pushing
    #[arg(short = 'u', long)]
    pub update: bool,

    /// Cleanup after push: checkout default branch, delete topic branch
    #[arg(short = 'f', long)]
    pub finish: bool,

    /// Use the push URL instead of the fetch URL
    #[arg(long)]
    pub use_pushurl: bool,

    /// Disable thin pack for push
    #[arg(long)]
    pub no_thin: bool,

    /// Execute a remote hook after push
    #[arg(long)]
    pub remote_hook: bool,

    /// Do not run custom scripts
    #[arg(long)]
    pub no_custom_script: bool,
}

/// Attempt to parse a Gerrit change URL into a `"CHANGE[,PS]"` string.
///
/// Supported URL patterns:
/// - `https://review.example.com/12345` -> `"12345"`
/// - `https://review.example.com/12345/2` -> `"12345,2"`
/// - `https://review.example.com/#/c/12345` -> `"12345"`
/// - `https://review.example.com/c/project/+/12345/1` -> `"12345,1"`
///
/// Returns `None` if the input is not a recognized URL pattern.
pub fn parse_change_url(input: &str) -> Option<String> {
    let url = url::Url::parse(input).ok()?;

    // Pattern: fragment-based /#/c/CHANGE[/PS]
    if let Some(fragment) = url.fragment() {
        let frag = fragment.trim_start_matches('/');
        if let Some(rest) = frag.strip_prefix("c/") {
            return parse_numeric_path_segments(rest);
        }
    }

    let path = url.path().trim_end_matches('/');

    // Pattern: /c/PROJECT/+/CHANGE[/PS]
    if let Some(rest) = path.strip_prefix("/c/") {
        if let Some(plus_pos) = rest.find("/+/") {
            let after_plus = &rest[plus_pos + 3..];
            return parse_numeric_path_segments(after_plus);
        }
    }

    // Pattern: /CHANGE[/PS] (trailing numeric segments)
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        [.., change, ps] if is_numeric(change) && is_numeric(ps) => Some(format!("{change},{ps}")),
        [.., change] if is_numeric(change) => Some(change.to_string()),
        _ => None,
    }
}

/// Normalize a download/cherrypick argument: if it's a URL, extract `"CHANGE[,PS]"`.
/// If it's already a change number or `"CHANGE,PS"`, return as-is.
pub fn normalize_change_arg(input: &str) -> String {
    parse_change_url(input).unwrap_or_else(|| input.to_string())
}

fn parse_numeric_path_segments(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        [change, ps, ..] if is_numeric(change) && is_numeric(ps) => Some(format!("{change},{ps}")),
        [change, ..] if is_numeric(change) => Some(change.to_string()),
        _ => None,
    }
}

fn is_numeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Parse a "CHANGE[,PS]" string into (change_id, optional_patchset).
///
/// - `"12345"` -> `("12345", None)`
/// - `"12345,2"` -> `("12345", Some(2))`
/// - `"12345,abc"` -> `("12345,abc", None)` (invalid patchset, treat as plain ID)
pub fn parse_change_patchset(input: &str) -> (String, Option<i32>) {
    if let Some((change, ps_str)) = input.split_once(',') {
        if let Ok(ps) = ps_str.parse::<i32>() {
            return (change.to_string(), Some(ps));
        }
    }
    (input.to_string(), None)
}

/// Find the target revision from a change's revision map.
///
/// If `patchset` is `Some(n)`, finds the revision with that patchset number.
/// If `patchset` is `None`, uses the change's `current_revision`.
pub fn find_target_revision(
    change: &ChangeInfo,
    patchset: Option<i32>,
) -> Result<(&str, &RevisionInfo)> {
    let revisions = change
        .revisions
        .as_ref()
        .context("change has no revision data")?;

    match patchset {
        Some(ps) => {
            for (sha, rev) in revisions {
                if rev.number == Some(ps) {
                    return Ok((sha, rev));
                }
            }
            anyhow::bail!("patchset {} not found in change", ps)
        }
        None => {
            let current = change
                .current_revision
                .as_deref()
                .context("change has no current revision")?;
            let rev = revisions
                .get(current)
                .context("current revision not found in revision map")?;
            Ok((current, rev))
        }
    }
}

/// Determine the local branch name for a downloaded change.
///
/// Uses `review/<owner>/<topic>` when both are available,
/// otherwise falls back to `review/<change_number>/<patchset>`.
pub fn download_branch_name(change: &ChangeInfo, patchset: i32) -> String {
    if let Some(ref topic) = change.topic {
        if let Some(ref owner) = change.owner {
            if let Some(ref username) = owner.username {
                return format!("review/{username}/{topic}");
            }
            if let Some(ref name) = owner.name {
                let sanitized = name.replace(' ', "_");
                return format!("review/{sanitized}/{topic}");
            }
        }
    }

    let change_num = change.number.unwrap_or(0);
    format!("review/{change_num}/{patchset}")
}

/// Download a change from Gerrit: fetch the ref and create a local branch.
pub async fn cmd_review_download(app: &mut App, change_arg: &str) -> Result<()> {
    let normalized = normalize_change_arg(change_arg);
    let (change_id, patchset) = parse_change_patchset(&normalized);

    app.authenticate_and_verify().await?;

    debug!("fetching change {} (patchset: {:?})", change_id, patchset);
    let change = app.gerrit.get_change_all_revisions(&change_id).await?;
    let (_, revision) = find_target_revision(&change, patchset)?;
    let ps_num = revision.number.context("revision has no patchset number")?;
    let git_ref = revision.git_ref.as_deref().context("revision has no ref")?;

    let remote = app.config.remote.clone();
    let root = app.git.root()?;
    let branch = download_branch_name(&change, ps_num);

    eprintln!(
        "Downloading {} patchset {} into {branch}...",
        change_id, ps_num
    );
    subprocess::git_fetch_ref(&remote, git_ref, &root)?;
    subprocess::git_checkout_new_branch(&branch, "FETCH_HEAD", &root)?;
    eprintln!("Switched to new branch '{branch}'");

    Ok(())
}

/// Cherry-pick a change onto the current branch.
pub async fn cmd_review_cherrypick(app: &mut App, change_arg: &str) -> Result<()> {
    let normalized = normalize_change_arg(change_arg);
    let (change_id, patchset) = parse_change_patchset(&normalized);

    app.authenticate_and_verify().await?;

    let change = app.gerrit.get_change_all_revisions(&change_id).await?;
    let (_, revision) = find_target_revision(&change, patchset)?;
    let git_ref = revision.git_ref.as_deref().context("revision has no ref")?;

    let remote = app.config.remote.clone();
    let root = app.git.root()?;

    eprintln!("Cherry-picking change {}...", change_id);
    subprocess::git_fetch_ref(&remote, git_ref, &root)?;
    subprocess::git_cherry_pick("FETCH_HEAD", &root)?;
    eprintln!("Cherry-pick applied.");

    Ok(())
}

/// Cherry-pick with "(cherry picked from commit ...)" indication.
pub async fn cmd_review_cherrypickindicate(app: &mut App, change_arg: &str) -> Result<()> {
    let normalized = normalize_change_arg(change_arg);
    let (change_id, patchset) = parse_change_patchset(&normalized);

    app.authenticate_and_verify().await?;

    let change = app.gerrit.get_change_all_revisions(&change_id).await?;
    let (_, revision) = find_target_revision(&change, patchset)?;
    let git_ref = revision.git_ref.as_deref().context("revision has no ref")?;

    let remote = app.config.remote.clone();
    let root = app.git.root()?;

    eprintln!("Cherry-picking change {} (with indication)...", change_id);
    subprocess::git_fetch_ref(&remote, git_ref, &root)?;
    subprocess::git_cherry_pick_indicate("FETCH_HEAD", &root)?;
    eprintln!("Cherry-pick applied with cherry-picked-from indication.");

    Ok(())
}

/// Cherry-pick without committing (apply to working directory only).
pub async fn cmd_review_cherrypickonly(app: &mut App, change_arg: &str) -> Result<()> {
    let normalized = normalize_change_arg(change_arg);
    let (change_id, patchset) = parse_change_patchset(&normalized);

    app.authenticate_and_verify().await?;

    let change = app.gerrit.get_change_all_revisions(&change_id).await?;
    let (_, revision) = find_target_revision(&change, patchset)?;
    let git_ref = revision.git_ref.as_deref().context("revision has no ref")?;

    let remote = app.config.remote.clone();
    let root = app.git.root()?;

    eprintln!(
        "Applying change {} to working directory (no commit)...",
        change_id
    );
    subprocess::git_fetch_ref(&remote, git_ref, &root)?;
    subprocess::git_cherry_pick_no_commit("FETCH_HEAD", &root)?;
    eprintln!("Change applied to working directory.");

    Ok(())
}

/// Parse a compare argument: `"CHANGE,PS[-PS]"`.
///
/// - `"12345,1-3"` → `("12345", 1, Some(3))`
/// - `"12345,1"` → `("12345", 1, None)` (diff patchset against current revision)
///
/// Returns an error if the format is invalid.
pub fn parse_compare_arg(input: &str) -> Result<(String, i32, Option<i32>)> {
    let (change, ps_part) = input
        .split_once(',')
        .context("compare argument must be CHANGE,PS[-PS]")?;

    if change.is_empty() {
        anyhow::bail!("compare argument has empty change number");
    }

    if let Some((from_str, to_str)) = ps_part.split_once('-') {
        let from: i32 = from_str
            .parse()
            .context("invalid 'from' patchset number in compare argument")?;
        let to: i32 = to_str
            .parse()
            .context("invalid 'to' patchset number in compare argument")?;
        Ok((change.to_string(), from, Some(to)))
    } else {
        let from: i32 = ps_part
            .parse()
            .context("invalid patchset number in compare argument")?;
        Ok((change.to_string(), from, None))
    }
}

/// Compare two patchsets of a change by diffing their fetched refs.
///
/// When `ps_to` is `None`, diffs against the change's current revision.
pub async fn cmd_review_compare(app: &mut App, compare_arg: &str) -> Result<()> {
    let (change_id, ps_from, ps_to) = parse_compare_arg(compare_arg)?;

    app.authenticate_and_verify().await?;

    debug!(
        "comparing change {} patchset {} vs {:?}",
        change_id, ps_from, ps_to
    );
    let change = app.gerrit.get_change_all_revisions(&change_id).await?;

    let (_, rev_from) = find_target_revision(&change, Some(ps_from))?;
    let ref_from = rev_from
        .git_ref
        .as_deref()
        .context("source revision has no ref")?
        .to_string();

    let (_, rev_to) = find_target_revision(&change, ps_to)?;
    let ps_to_num = rev_to.number.unwrap_or(0);
    let ref_to = rev_to
        .git_ref
        .as_deref()
        .context("target revision has no ref")?
        .to_string();

    let remote = app.config.remote.clone();
    let root = app.git.root()?;

    eprintln!(
        "Comparing change {} patchset {} vs {}...",
        change_id, ps_from, ps_to_num
    );
    let sha_from = subprocess::git_fetch_ref_sha(&remote, &ref_from, &root)?;
    let sha_to = subprocess::git_fetch_ref_sha(&remote, &ref_to, &root)?;
    subprocess::git_diff(&sha_from, &sha_to, &root)?;

    Ok(())
}

/// List open changes on Gerrit.
///
/// Queries `status:open project:<project>` (and `branch:<branch>` if specified).
/// Brief mode (`-l`) shows number, branch, subject.
/// Verbose mode (`-ll`) adds a topic column.
pub async fn cmd_review_list(app: &App, branch: Option<&str>, verbose: bool) -> Result<()> {
    let query = list::build_list_query(&app.config.project, branch);
    debug!("listing changes with query: {}", query);

    let changes = app.gerrit.query_changes(&query).await?;

    if changes.is_empty() {
        return Ok(());
    }

    let output = if verbose {
        list::format_reviews_verbose(&changes)
    } else {
        list::format_reviews_text(&changes)
    };

    print!("{output}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use clap::Parser;

    // Wrapper to parse ReviewArgs from a flat command line, simulating `git-review ...`
    #[derive(Parser)]
    #[command(name = "test")]
    struct TestCli {
        #[command(flatten)]
        review: ReviewArgs,
    }

    fn parse_review(args: &[&str]) -> ReviewArgs {
        let mut full_args = vec!["test"];
        full_args.extend_from_slice(args);
        TestCli::parse_from(full_args).review
    }

    fn try_parse_review(args: &[&str]) -> Result<ReviewArgs, clap::Error> {
        let mut full_args = vec!["test"];
        full_args.extend_from_slice(args);
        TestCli::try_parse_from(full_args).map(|c| c.review)
    }

    // === Positional branch ===

    #[test]
    fn parse_branch_positional() {
        let args = parse_review(&["main"]);
        assert_eq!(args.branch.as_deref(), Some("main"));
    }

    #[test]
    fn parse_no_args() {
        let args = parse_review(&[]);
        assert!(args.branch.is_none());
        assert!(args.download.is_none());
        assert!(!args.setup);
        assert_eq!(args.list, 0);
    }

    // === Mode flags: short and long forms ===

    #[test]
    fn parse_download_short() {
        let args = parse_review(&["-d", "12345"]);
        assert_eq!(args.download.as_deref(), Some("12345"));
    }

    #[test]
    fn parse_download_long() {
        let args = parse_review(&["--download", "12345"]);
        assert_eq!(args.download.as_deref(), Some("12345"));
    }

    #[test]
    fn parse_download_with_patchset() {
        let args = parse_review(&["-d", "12345,2"]);
        assert_eq!(args.download.as_deref(), Some("12345,2"));
    }

    #[test]
    fn parse_cherrypick_short() {
        let args = parse_review(&["-x", "12345"]);
        assert_eq!(args.cherrypick.as_deref(), Some("12345"));
    }

    #[test]
    fn parse_cherrypick_long() {
        let args = parse_review(&["--cherrypick", "12345"]);
        assert_eq!(args.cherrypick.as_deref(), Some("12345"));
    }

    #[test]
    fn parse_cherrypickindicate_short() {
        let args = parse_review(&["-X", "12345"]);
        assert_eq!(args.cherrypickindicate.as_deref(), Some("12345"));
    }

    #[test]
    fn parse_cherrypickindicate_long() {
        let args = parse_review(&["--cherrypickindicate", "12345"]);
        assert_eq!(args.cherrypickindicate.as_deref(), Some("12345"));
    }

    #[test]
    fn parse_cherrypickonly_short() {
        let args = parse_review(&["-N", "12345"]);
        assert_eq!(args.cherrypickonly.as_deref(), Some("12345"));
    }

    #[test]
    fn parse_cherrypickonly_long() {
        let args = parse_review(&["--cherrypickonly", "12345"]);
        assert_eq!(args.cherrypickonly.as_deref(), Some("12345"));
    }

    #[test]
    fn parse_compare_short() {
        let args = parse_review(&["-m", "12345,1-3"]);
        assert_eq!(args.compare.as_deref(), Some("12345,1-3"));
    }

    #[test]
    fn parse_compare_long() {
        let args = parse_review(&["--compare", "12345,1"]);
        assert_eq!(args.compare.as_deref(), Some("12345,1"));
    }

    #[test]
    fn parse_list_short() {
        let args = parse_review(&["-l"]);
        assert_eq!(args.list, 1);
    }

    #[test]
    fn parse_list_verbose() {
        let args = parse_review(&["-l", "-l"]);
        assert_eq!(args.list, 2);
    }

    #[test]
    fn parse_list_long() {
        let args = parse_review(&["--list"]);
        assert_eq!(args.list, 1);
    }

    #[test]
    fn parse_setup_short() {
        let args = parse_review(&["-s"]);
        assert!(args.setup);
    }

    #[test]
    fn parse_setup_long() {
        let args = parse_review(&["--setup"]);
        assert!(args.setup);
    }

    // === Mutually exclusive mode flags ===

    #[test]
    fn mode_download_and_list_conflict() {
        let result = try_parse_review(&["-d", "12345", "-l"]);
        assert!(result.is_err(), "download and list should conflict");
    }

    #[test]
    fn mode_download_and_setup_conflict() {
        let result = try_parse_review(&["-d", "12345", "-s"]);
        assert!(result.is_err(), "download and setup should conflict");
    }

    #[test]
    fn mode_cherrypick_and_compare_conflict() {
        let result = try_parse_review(&["-x", "12345", "-m", "12345,1-3"]);
        assert!(result.is_err(), "cherrypick and compare should conflict");
    }

    #[test]
    fn mode_list_and_setup_conflict() {
        let result = try_parse_review(&["-l", "-s"]);
        assert!(result.is_err(), "list and setup should conflict");
    }

    // === Mutually exclusive pairs ===

    #[test]
    fn topic_and_no_topic_conflict() {
        let result = try_parse_review(&["-t", "my-topic", "-T"]);
        assert!(result.is_err(), "topic and no-topic should conflict");
    }

    #[test]
    fn no_rebase_and_force_rebase_conflict() {
        let result = try_parse_review(&["-R", "-F"]);
        assert!(
            result.is_err(),
            "no-rebase and force-rebase should conflict"
        );
    }

    #[test]
    fn track_and_no_track_conflict() {
        let result = try_parse_review(&["--track", "--no-track"]);
        assert!(result.is_err(), "track and no-track should conflict");
    }

    #[test]
    fn wip_and_ready_conflict() {
        let result = try_parse_review(&["-w", "-W"]);
        assert!(result.is_err(), "wip and ready should conflict");
    }

    #[test]
    fn private_and_remove_private_conflict() {
        let result = try_parse_review(&["-p", "-P"]);
        assert!(
            result.is_err(),
            "private and remove-private should conflict"
        );
    }

    // === Topic flags ===

    #[test]
    fn parse_topic_short() {
        let args = parse_review(&["-t", "my-topic"]);
        assert_eq!(args.topic.as_deref(), Some("my-topic"));
    }

    #[test]
    fn parse_topic_long() {
        let args = parse_review(&["--topic", "my-topic"]);
        assert_eq!(args.topic.as_deref(), Some("my-topic"));
    }

    #[test]
    fn parse_no_topic_short() {
        let args = parse_review(&["-T"]);
        assert!(args.no_topic);
    }

    #[test]
    fn parse_no_topic_long() {
        let args = parse_review(&["--no-topic"]);
        assert!(args.no_topic);
    }

    // === Rebase flags ===

    #[test]
    fn parse_no_rebase_short() {
        let args = parse_review(&["-R"]);
        assert!(args.no_rebase);
    }

    #[test]
    fn parse_no_rebase_long() {
        let args = parse_review(&["--no-rebase"]);
        assert!(args.no_rebase);
    }

    #[test]
    fn parse_force_rebase_short() {
        let args = parse_review(&["-F"]);
        assert!(args.force_rebase);
    }

    #[test]
    fn parse_force_rebase_long() {
        let args = parse_review(&["--force-rebase"]);
        assert!(args.force_rebase);
    }

    // === WIP/Ready flags ===

    #[test]
    fn parse_wip_short() {
        let args = parse_review(&["-w"]);
        assert!(args.wip);
    }

    #[test]
    fn parse_wip_long() {
        let args = parse_review(&["--wip"]);
        assert!(args.wip);
    }

    #[test]
    fn parse_work_in_progress_alias() {
        let args = parse_review(&["--work-in-progress"]);
        assert!(args.wip);
    }

    #[test]
    fn parse_ready_short() {
        let args = parse_review(&["-W"]);
        assert!(args.ready);
    }

    #[test]
    fn parse_ready_long() {
        let args = parse_review(&["--ready"]);
        assert!(args.ready);
    }

    // === Privacy flags ===

    #[test]
    fn parse_private_short() {
        let args = parse_review(&["-p"]);
        assert!(args.private);
    }

    #[test]
    fn parse_private_long() {
        let args = parse_review(&["--private"]);
        assert!(args.private);
    }

    #[test]
    fn parse_remove_private_short() {
        let args = parse_review(&["-P"]);
        assert!(args.remove_private);
    }

    #[test]
    fn parse_remove_private_long() {
        let args = parse_review(&["--remove-private"]);
        assert!(args.remove_private);
    }

    // === Push metadata ===

    #[test]
    fn parse_reviewers() {
        let args = parse_review(&["--reviewers", "alice", "bob"]);
        assert_eq!(args.reviewers, vec!["alice", "bob"]);
    }

    #[test]
    fn parse_cc() {
        let args = parse_review(&["--cc", "carol"]);
        assert_eq!(args.cc, vec!["carol"]);
    }

    #[test]
    fn parse_hashtags() {
        let args = parse_review(&["--hashtags", "urgent", "bug"]);
        assert_eq!(args.hashtags, vec!["urgent", "bug"]);
    }

    #[test]
    fn parse_notify() {
        let args = parse_review(&["--notify", "NONE"]);
        assert_eq!(args.notify.as_deref(), Some("NONE"));
    }

    #[test]
    fn parse_message() {
        let args = parse_review(&["--message", "fix the bug"]);
        assert_eq!(args.message.as_deref(), Some("fix the bug"));
    }

    // === Behavior flags ===

    #[test]
    fn parse_remote_short() {
        let args = parse_review(&["-r", "upstream"]);
        assert_eq!(args.remote.as_deref(), Some("upstream"));
    }

    #[test]
    fn parse_remote_long() {
        let args = parse_review(&["--remote", "upstream"]);
        assert_eq!(args.remote.as_deref(), Some("upstream"));
    }

    #[test]
    fn parse_dry_run_short() {
        let args = parse_review(&["-n"]);
        assert!(args.dry_run);
    }

    #[test]
    fn parse_dry_run_long() {
        let args = parse_review(&["--dry-run"]);
        assert!(args.dry_run);
    }

    #[test]
    fn parse_new_changeid_short() {
        let args = parse_review(&["-i"]);
        assert!(args.new_changeid);
    }

    #[test]
    fn parse_new_changeid_long() {
        let args = parse_review(&["--new-changeid"]);
        assert!(args.new_changeid);
    }

    #[test]
    fn parse_yes_short() {
        let args = parse_review(&["-y"]);
        assert!(args.yes);
    }

    #[test]
    fn parse_yes_long() {
        let args = parse_review(&["--yes"]);
        assert!(args.yes);
    }

    #[test]
    fn parse_update_short() {
        let args = parse_review(&["-u"]);
        assert!(args.update);
    }

    #[test]
    fn parse_update_long() {
        let args = parse_review(&["--update"]);
        assert!(args.update);
    }

    #[test]
    fn parse_finish_short() {
        let args = parse_review(&["-f"]);
        assert!(args.finish);
    }

    #[test]
    fn parse_finish_long() {
        let args = parse_review(&["--finish"]);
        assert!(args.finish);
    }

    #[test]
    fn parse_use_pushurl() {
        let args = parse_review(&["--use-pushurl"]);
        assert!(args.use_pushurl);
    }

    #[test]
    fn parse_no_thin() {
        let args = parse_review(&["--no-thin"]);
        assert!(args.no_thin);
    }

    #[test]
    fn parse_remote_hook() {
        let args = parse_review(&["--remote-hook"]);
        assert!(args.remote_hook);
    }

    #[test]
    fn parse_no_custom_script() {
        let args = parse_review(&["--no-custom-script"]);
        assert!(args.no_custom_script);
    }

    // === Track flags ===

    #[test]
    fn parse_track() {
        let args = parse_review(&["--track"]);
        assert!(args.track);
    }

    #[test]
    fn parse_no_track() {
        let args = parse_review(&["--no-track"]);
        assert!(args.no_track);
    }

    // === Combined flags ===

    #[test]
    fn parse_push_with_all_options() {
        let args = parse_review(&[
            "-w",
            "-t",
            "my-topic",
            "-r",
            "origin",
            "--reviewers",
            "alice",
            "--cc",
            "bob",
            "--hashtags",
            "urgent",
            "--notify",
            "ALL",
            "--message",
            "ready",
            "-n",
            "-R",
            "main",
        ]);
        assert!(args.wip);
        assert_eq!(args.topic.as_deref(), Some("my-topic"));
        assert_eq!(args.remote.as_deref(), Some("origin"));
        assert_eq!(args.reviewers, vec!["alice"]);
        assert_eq!(args.cc, vec!["bob"]);
        assert_eq!(args.hashtags, vec!["urgent"]);
        assert_eq!(args.notify.as_deref(), Some("ALL"));
        assert_eq!(args.message.as_deref(), Some("ready"));
        assert!(args.dry_run);
        assert!(args.no_rebase);
        assert_eq!(args.branch.as_deref(), Some("main"));
    }

    // === URL parsing ===

    #[test]
    fn url_parse_simple_change() {
        let result = parse_change_url("https://review.example.com/12345");
        assert_eq!(result.as_deref(), Some("12345"));
    }

    #[test]
    fn url_parse_change_with_patchset() {
        let result = parse_change_url("https://review.example.com/12345/2");
        assert_eq!(result.as_deref(), Some("12345,2"));
    }

    #[test]
    fn url_parse_fragment_change() {
        let result = parse_change_url("https://review.example.com/#/c/12345");
        assert_eq!(result.as_deref(), Some("12345"));
    }

    #[test]
    fn url_parse_polygerrit_change_with_patchset() {
        let result = parse_change_url("https://review.example.com/c/project/+/12345/1");
        assert_eq!(result.as_deref(), Some("12345,1"));
    }

    #[test]
    fn url_parse_polygerrit_change_no_patchset() {
        let result = parse_change_url("https://review.example.com/c/my/project/+/12345");
        assert_eq!(result.as_deref(), Some("12345"));
    }

    #[test]
    fn url_parse_not_a_url() {
        let result = parse_change_url("12345");
        assert!(result.is_none());
    }

    #[test]
    fn url_parse_no_numeric_segments() {
        let result = parse_change_url("https://review.example.com/dashboard/self");
        assert!(result.is_none());
    }

    #[test]
    fn normalize_url_to_change_id() {
        assert_eq!(
            normalize_change_arg("https://review.example.com/12345"),
            "12345"
        );
    }

    #[test]
    fn normalize_passthrough_change_number() {
        assert_eq!(normalize_change_arg("12345"), "12345");
    }

    #[test]
    fn normalize_passthrough_change_with_patchset() {
        assert_eq!(normalize_change_arg("12345,2"), "12345,2");
    }

    // === parse_change_patchset ===

    #[test]
    fn parse_change_patchset_number_only() {
        let (change, ps) = parse_change_patchset("12345");
        assert_eq!(change, "12345");
        assert_eq!(ps, None);
    }

    #[test]
    fn parse_change_patchset_with_ps() {
        let (change, ps) = parse_change_patchset("12345,2");
        assert_eq!(change, "12345");
        assert_eq!(ps, Some(2));
    }

    #[test]
    fn parse_change_patchset_invalid_ps() {
        let (change, ps) = parse_change_patchset("12345,abc");
        assert_eq!(change, "12345,abc");
        assert_eq!(ps, None);
    }

    #[test]
    fn parse_change_patchset_change_id_string() {
        let (change, ps) = parse_change_patchset("Iabcdef1234");
        assert_eq!(change, "Iabcdef1234");
        assert_eq!(ps, None);
    }

    // === find_target_revision ===

    fn make_test_change() -> ChangeInfo {
        let mut revisions = HashMap::new();
        revisions.insert(
            "abc123".to_string(),
            RevisionInfo {
                number: Some(1),
                git_ref: Some("refs/changes/45/12345/1".to_string()),
                commit: None,
            },
        );
        revisions.insert(
            "def456".to_string(),
            RevisionInfo {
                number: Some(2),
                git_ref: Some("refs/changes/45/12345/2".to_string()),
                commit: None,
            },
        );

        ChangeInfo {
            id: None,
            project: Some("proj".to_string()),
            branch: Some("main".to_string()),
            change_id: Some("Iabcdef".to_string()),
            subject: Some("Fix bug".to_string()),
            status: Some("NEW".to_string()),
            topic: Some("my-feature".to_string()),
            created: None,
            updated: None,
            number: Some(12345),
            owner: Some(crate::gerrit::AccountInfo {
                account_id: 1000096,
                name: Some("Alice".to_string()),
                email: None,
                username: Some("alice".to_string()),
                display_name: None,
            }),
            current_revision: Some("def456".to_string()),
            revisions: Some(revisions),
            messages: None,
            insertions: None,
            deletions: None,
        }
    }

    #[test]
    fn find_revision_by_patchset() {
        let change = make_test_change();
        let (sha, rev) = find_target_revision(&change, Some(1)).unwrap();
        assert_eq!(sha, "abc123");
        assert_eq!(rev.number, Some(1));
        assert_eq!(rev.git_ref.as_deref(), Some("refs/changes/45/12345/1"));
    }

    #[test]
    fn find_revision_current() {
        let change = make_test_change();
        let (sha, rev) = find_target_revision(&change, None).unwrap();
        assert_eq!(sha, "def456");
        assert_eq!(rev.number, Some(2));
    }

    #[test]
    fn find_revision_missing_patchset() {
        let change = make_test_change();
        let result = find_target_revision(&change, Some(99));
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("patchset 99"),
            "error should mention missing patchset"
        );
    }

    #[test]
    fn find_revision_no_revisions() {
        let mut change = make_test_change();
        change.revisions = None;
        let result = find_target_revision(&change, None);
        assert!(result.is_err());
    }

    // === download_branch_name ===

    #[test]
    fn download_branch_with_topic_and_owner() {
        let change = make_test_change();
        assert_eq!(download_branch_name(&change, 2), "review/alice/my-feature");
    }

    #[test]
    fn download_branch_no_topic() {
        let mut change = make_test_change();
        change.topic = None;
        assert_eq!(download_branch_name(&change, 2), "review/12345/2");
    }

    #[test]
    fn download_branch_no_owner() {
        let mut change = make_test_change();
        change.owner = None;
        assert_eq!(download_branch_name(&change, 1), "review/12345/1");
    }

    #[test]
    fn download_branch_owner_name_only() {
        let mut change = make_test_change();
        if let Some(ref mut owner) = change.owner {
            owner.username = None;
            owner.name = Some("Alice Smith".to_string());
        }
        assert_eq!(
            download_branch_name(&change, 2),
            "review/Alice_Smith/my-feature"
        );
    }

    // === parse_compare_arg ===

    #[test]
    fn compare_arg_range() {
        let (change, from, to) = parse_compare_arg("12345,1-3").unwrap();
        assert_eq!(change, "12345");
        assert_eq!(from, 1);
        assert_eq!(to, Some(3));
    }

    #[test]
    fn compare_arg_single_patchset() {
        let (change, from, to) = parse_compare_arg("12345,1").unwrap();
        assert_eq!(change, "12345");
        assert_eq!(from, 1);
        assert_eq!(to, None);
    }

    #[test]
    fn compare_arg_no_comma() {
        let result = parse_compare_arg("12345");
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("CHANGE,PS"),
            "error should mention expected format"
        );
    }

    #[test]
    fn compare_arg_invalid_patchset() {
        let result = parse_compare_arg("12345,abc");
        assert!(result.is_err());
    }

    #[test]
    fn compare_arg_invalid_range_to() {
        let result = parse_compare_arg("12345,1-abc");
        assert!(result.is_err());
    }

    #[test]
    fn compare_arg_empty_change() {
        let result = parse_compare_arg(",1-3");
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("empty"),
            "error should mention empty change number"
        );
    }
}
