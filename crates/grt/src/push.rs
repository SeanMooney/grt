// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use anyhow::{Context, Result};
use serde::Serialize;

/// Structured result from a push operation.
#[derive(Debug, Serialize)]
pub struct PushResult {
    pub commits: usize,
    pub remote: String,
    pub branch: String,
    pub change_id: Option<String>,
    pub refspec: String,
}

/// Options for building a Gerrit push refspec.
#[derive(Debug, Default)]
pub struct PushOptions {
    pub branch: String,
    pub topic: Option<String>,
    pub wip: bool,
    pub ready: bool,
    pub private: bool,
    pub remove_private: bool,
    pub reviewers: Vec<String>,
    pub cc: Vec<String>,
    pub hashtags: Vec<String>,
    pub message: Option<String>,
    pub notify: Option<String>,
}

/// Build the refspec for `git push`, e.g. `HEAD:refs/for/main%topic=foo,r=alice`.
pub fn build_refspec(opts: &PushOptions) -> Result<String> {
    let mut options: Vec<String> = Vec::new();

    if let Some(ref topic) = opts.topic {
        if topic != &opts.branch {
            options.push(format!("topic={topic}"));
        }
    }

    if opts.wip {
        options.push("wip".to_string());
    }

    if opts.ready {
        options.push("ready".to_string());
    }

    if opts.private {
        options.push("private".to_string());
    }

    if opts.remove_private {
        options.push("remove-private".to_string());
    }

    for reviewer in &opts.reviewers {
        let trimmed = reviewer.trim();
        if trimmed.contains(char::is_whitespace) {
            anyhow::bail!("reviewer name contains whitespace: {trimmed:?}");
        }
        options.push(format!("r={trimmed}"));
    }

    for cc in &opts.cc {
        let trimmed = cc.trim();
        if trimmed.contains(char::is_whitespace) {
            anyhow::bail!("CC name contains whitespace: {trimmed:?}");
        }
        options.push(format!("cc={trimmed}"));
    }

    for hashtag in &opts.hashtags {
        let trimmed = hashtag.trim();
        if trimmed.contains(char::is_whitespace) {
            anyhow::bail!("hashtag contains whitespace: {trimmed:?}");
        }
        options.push(format!("hashtag={trimmed}"));
    }

    if let Some(ref message) = opts.message {
        let encoded = urlencoding::encode(message);
        options.push(format!("m={encoded}"));
    }

    if let Some(ref notify) = opts.notify {
        options.push(format!("notify={notify}"));
    }

    let refspec = if options.is_empty() {
        format!("HEAD:refs/for/{}", opts.branch)
    } else {
        format!("HEAD:refs/for/{}%{}", opts.branch, options.join(","))
    };

    Ok(refspec)
}

/// Extract the Change-Id trailer value from a commit message.
/// Returns `Some("I<hex>")` if found, `None` otherwise.
pub fn extract_change_id(commit_message: &str) -> Option<String> {
    for line in commit_message.lines().rev() {
        let trimmed = line.trim();
        if let Some(id) = trimmed.strip_prefix("Change-Id: ") {
            let id = id.trim();
            if id.starts_with('I')
                && id.len() == 41
                && id[1..].chars().all(|c| c.is_ascii_hexdigit())
            {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Status of the Change-Id in a commit message.
#[derive(Debug, PartialEq, Eq)]
pub enum ChangeIdStatus {
    /// Change-Id is present and valid.
    Present(String),
    /// Change-Id is missing but can be auto-amended (single commit, hook installed).
    MissingCanAutoAmend,
    /// Change-Id is missing and hook is not installed (need setup first).
    MissingNeedHook,
}

/// Check the Change-Id status of a commit message.
///
/// Returns the appropriate status based on whether the Change-Id is present
/// and whether the hook is installed (for auto-amend capability).
pub fn check_change_id_status(commit_message: &str, hook_installed: bool) -> ChangeIdStatus {
    if let Some(id) = extract_change_id(commit_message) {
        ChangeIdStatus::Present(id)
    } else if hook_installed {
        ChangeIdStatus::MissingCanAutoAmend
    } else {
        ChangeIdStatus::MissingNeedHook
    }
}

/// Validate that the HEAD commit contains a Change-Id trailer.
pub fn validate_change_id(commit_message: &str) -> Result<String> {
    extract_change_id(commit_message)
        .context("HEAD commit is missing a Change-Id trailer. Run `grt setup` to install the commit-msg hook, then amend the commit")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(branch: &str) -> PushOptions {
        PushOptions {
            branch: branch.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn build_refspec_basic() {
        let refspec = build_refspec(&opts("main")).unwrap();
        assert_eq!(refspec, "HEAD:refs/for/main");
    }

    #[test]
    fn build_refspec_with_topic() {
        let mut o = opts("main");
        o.topic = Some("my-feature".to_string());
        let refspec = build_refspec(&o).unwrap();
        assert_eq!(refspec, "HEAD:refs/for/main%topic=my-feature");
    }

    #[test]
    fn build_refspec_with_wip() {
        let mut o = opts("main");
        o.wip = true;
        let refspec = build_refspec(&o).unwrap();
        assert_eq!(refspec, "HEAD:refs/for/main%wip");
    }

    #[test]
    fn build_refspec_with_reviewers() {
        let mut o = opts("main");
        o.reviewers = vec!["alice".into(), "bob".into()];
        let refspec = build_refspec(&o).unwrap();
        assert_eq!(refspec, "HEAD:refs/for/main%r=alice,r=bob");
    }

    #[test]
    fn build_refspec_with_cc() {
        let mut o = opts("main");
        o.cc = vec!["carol".into()];
        let refspec = build_refspec(&o).unwrap();
        assert_eq!(refspec, "HEAD:refs/for/main%cc=carol");
    }

    #[test]
    fn build_refspec_all_options() {
        let mut o = opts("develop");
        o.topic = Some("feature-x".to_string());
        o.wip = true;
        o.reviewers = vec!["alice".into()];
        o.cc = vec!["bob".into()];
        o.hashtags = vec!["urgent".into()];
        let refspec = build_refspec(&o).unwrap();
        assert_eq!(
            refspec,
            "HEAD:refs/for/develop%topic=feature-x,wip,r=alice,cc=bob,hashtag=urgent"
        );
    }

    #[test]
    fn build_refspec_message_url_encoded() {
        let mut o = opts("main");
        o.message = Some("fix the bug".to_string());
        let refspec = build_refspec(&o).unwrap();
        assert!(refspec.contains("m=fix%20the%20bug"), "refspec: {refspec}");
    }

    #[test]
    fn build_refspec_rejects_whitespace_in_reviewer() {
        let mut o = opts("main");
        o.reviewers = vec!["alice bob".into()];
        let result = build_refspec(&o);
        assert!(result.is_err());
    }

    #[test]
    fn build_refspec_topic_same_as_branch_skipped() {
        let mut o = opts("main");
        o.topic = Some("main".to_string());
        let refspec = build_refspec(&o).unwrap();
        assert_eq!(refspec, "HEAD:refs/for/main");
    }

    #[test]
    fn detect_change_id_present() {
        let msg = "Fix bug\n\nSome description.\n\nChange-Id: I1234567890abcdef1234567890abcdef12345678\n";
        let id = extract_change_id(msg);
        assert_eq!(
            id.as_deref(),
            Some("I1234567890abcdef1234567890abcdef12345678")
        );
    }

    #[test]
    fn detect_change_id_absent() {
        let msg = "Fix bug\n\nSome description.\n";
        assert!(extract_change_id(msg).is_none());
    }

    #[test]
    fn detect_change_id_multiple_trailers() {
        let msg = "Fix bug\n\nSigned-off-by: Alice <alice@example.com>\nChange-Id: Iabcdef1234567890abcdef1234567890abcdef12\nReviewed-by: Bob <bob@example.com>\n";
        let id = extract_change_id(msg);
        assert_eq!(
            id.as_deref(),
            Some("Iabcdef1234567890abcdef1234567890abcdef12")
        );
    }

    #[test]
    fn build_refspec_rejects_whitespace_in_cc() {
        let mut o = opts("main");
        o.cc = vec!["alice bob".into()];
        let result = build_refspec(&o);
        assert!(result.is_err());
    }

    #[test]
    fn build_refspec_rejects_whitespace_in_hashtag() {
        let mut o = opts("main");
        o.hashtags = vec!["my tag".into()];
        let result = build_refspec(&o);
        assert!(result.is_err());
    }

    #[test]
    fn check_change_id_status_present() {
        let msg = "Fix bug\n\nChange-Id: I1234567890abcdef1234567890abcdef12345678\n";
        let status = check_change_id_status(msg, false);
        assert_eq!(
            status,
            ChangeIdStatus::Present("I1234567890abcdef1234567890abcdef12345678".to_string())
        );
    }

    #[test]
    fn check_change_id_status_missing_can_auto_amend() {
        let msg = "Fix bug\n\nSome description.\n";
        let status = check_change_id_status(msg, true);
        assert_eq!(status, ChangeIdStatus::MissingCanAutoAmend);
    }

    #[test]
    fn check_change_id_status_missing_need_hook() {
        let msg = "Fix bug\n\nSome description.\n";
        let status = check_change_id_status(msg, false);
        assert_eq!(status, ChangeIdStatus::MissingNeedHook);
    }
}
