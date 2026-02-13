// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use crate::gerrit::ChangeInfo;

/// Build the Gerrit query string for listing open changes.
///
/// Always includes `status:open`. Adds `project:<project>` when non-empty,
/// and `branch:<branch>` when provided.
pub fn build_list_query(project: &str, branch: Option<&str>) -> String {
    let mut query = "status:open".to_string();
    if !project.is_empty() {
        query.push_str(&format!(" project:{project}"));
    }
    if let Some(branch) = branch {
        query.push_str(&format!(" branch:{branch}"));
    }
    query
}

/// Format a list of changes for brief output (`-l`).
///
/// Columns: right-aligned number, left-aligned branch, subject.
/// Returns empty string if changes is empty (matching git-review).
pub fn format_reviews_text(changes: &[ChangeInfo]) -> String {
    if changes.is_empty() {
        return String::new();
    }

    let num_width = max_number_width(changes);
    let branch_width = changes
        .iter()
        .map(|c| c.branch.as_deref().unwrap_or("").len())
        .max()
        .unwrap_or(0);

    let mut output = String::new();
    for change in changes {
        let num = change.number.unwrap_or(0);
        let branch = change.branch.as_deref().unwrap_or("");
        let subject = change.subject.as_deref().unwrap_or("");
        use std::fmt::Write;
        let _ = writeln!(
            output,
            "{num:>num_width$} {branch:<branch_width$} {subject}",
            num_width = num_width,
            branch_width = branch_width
        );
    }

    output
}

/// Format a list of changes for verbose output (`-ll`).
///
/// Columns: right-aligned number, left-aligned branch, left-aligned topic, subject.
/// Returns empty string if changes is empty (matching git-review).
pub fn format_reviews_verbose(changes: &[ChangeInfo]) -> String {
    if changes.is_empty() {
        return String::new();
    }

    let num_width = max_number_width(changes);
    let branch_width = changes
        .iter()
        .map(|c| c.branch.as_deref().unwrap_or("").len())
        .max()
        .unwrap_or(0);
    let topic_width = changes
        .iter()
        .map(|c| c.topic.as_deref().unwrap_or("").len())
        .max()
        .unwrap_or(0);

    let mut output = String::new();
    for change in changes {
        let num = change.number.unwrap_or(0);
        let branch = change.branch.as_deref().unwrap_or("");
        let topic = change.topic.as_deref().unwrap_or("");
        let subject = change.subject.as_deref().unwrap_or("");
        use std::fmt::Write;
        let _ = writeln!(
            output,
            "{num:>num_width$} {branch:<branch_width$} {topic:<topic_width$} {subject}",
            num_width = num_width,
            branch_width = branch_width,
            topic_width = topic_width
        );
    }

    output
}

/// Compute the maximum display width of change numbers in the list.
fn max_number_width(changes: &[ChangeInfo]) -> usize {
    changes
        .iter()
        .map(|c| {
            let n = c.number.unwrap_or(0);
            if n == 0 {
                1
            } else {
                format!("{n}").len()
            }
        })
        .max()
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gerrit::ChangeInfo;

    fn make_change(number: i64, branch: &str, subject: &str, topic: Option<&str>) -> ChangeInfo {
        ChangeInfo {
            id: None,
            project: Some("proj".to_string()),
            branch: Some(branch.to_string()),
            change_id: None,
            subject: Some(subject.to_string()),
            status: Some("NEW".to_string()),
            topic: topic.map(|t| t.to_string()),
            created: None,
            updated: None,
            number: Some(number),
            owner: None,
            current_revision: None,
            revisions: None,
            messages: None,
            insertions: None,
            deletions: None,
        }
    }

    // === build_list_query ===

    #[test]
    fn query_with_project() {
        let q = build_list_query("my/project", None);
        assert_eq!(q, "status:open project:my/project");
    }

    #[test]
    fn query_with_project_and_branch() {
        let q = build_list_query("my/project", Some("main"));
        assert_eq!(q, "status:open project:my/project branch:main");
    }

    #[test]
    fn query_empty_project() {
        let q = build_list_query("", None);
        assert_eq!(q, "status:open");
    }

    #[test]
    fn query_empty_project_with_branch() {
        let q = build_list_query("", Some("develop"));
        assert_eq!(q, "status:open branch:develop");
    }

    // === format_reviews_text (brief) ===

    #[test]
    fn text_empty_returns_empty() {
        assert_eq!(format_reviews_text(&[]), "");
    }

    #[test]
    fn text_single_change() {
        let changes = vec![make_change(12345, "main", "Fix the bug", None)];
        let output = format_reviews_text(&changes);
        assert_eq!(output, "12345 main Fix the bug\n");
    }

    #[test]
    fn text_multiple_changes_aligned() {
        let changes = vec![
            make_change(12345, "main", "Fix the bug", None),
            make_change(99, "feature/long-branch", "Add feature", None),
        ];
        let output = format_reviews_text(&changes);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2);
        // Number column should be right-aligned (width 5)
        assert!(lines[0].starts_with("12345 "));
        assert!(lines[1].starts_with("   99 "));
        // Branch column should be left-aligned (width of longest branch)
        assert!(lines[0].contains("main                "));
        assert!(lines[1].contains("feature/long-branch "));
    }

    #[test]
    fn text_three_changes_number_alignment() {
        let changes = vec![
            make_change(1, "main", "First", None),
            make_change(100, "main", "Second", None),
            make_change(99999, "main", "Third", None),
        ];
        let output = format_reviews_text(&changes);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3);
        // All numbers right-aligned to width 5
        assert!(lines[0].starts_with("    1 "));
        assert!(lines[1].starts_with("  100 "));
        assert!(lines[2].starts_with("99999 "));
    }

    // === format_reviews_verbose ===

    #[test]
    fn verbose_empty_returns_empty() {
        assert_eq!(format_reviews_verbose(&[]), "");
    }

    #[test]
    fn verbose_single_change_with_topic() {
        let changes = vec![make_change(12345, "main", "Fix the bug", Some("my-topic"))];
        let output = format_reviews_verbose(&changes);
        assert_eq!(output, "12345 main my-topic Fix the bug\n");
    }

    #[test]
    fn verbose_multiple_changes_aligned() {
        let changes = vec![
            make_change(12345, "main", "Fix the bug", Some("bugfix")),
            make_change(99, "develop", "Add feature", Some("new-feature")),
        ];
        let output = format_reviews_verbose(&changes);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2);
        // Number right-aligned to width 5
        assert!(lines[0].starts_with("12345 "));
        assert!(lines[1].starts_with("   99 "));
        // Topic column present and aligned
        assert!(lines[0].contains("bugfix      "));
        assert!(lines[1].contains("new-feature "));
    }

    #[test]
    fn verbose_missing_topics_shown_as_blank() {
        let changes = vec![
            make_change(100, "main", "Has topic", Some("my-topic")),
            make_change(200, "main", "No topic", None),
        ];
        let output = format_reviews_verbose(&changes);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2);
        // First line has topic
        assert!(lines[0].contains("my-topic"));
        // Second line has blank topic column but correct subject
        assert!(lines[1].contains("No topic"));
    }
}
