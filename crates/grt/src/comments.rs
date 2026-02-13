// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use std::collections::HashMap;
use std::fmt::Write as _;

use serde::Serialize;

use crate::gerrit::{ChangeInfo, ChangeMessageInfo, CommentInfo};

/// A thread of comments on a single location in a file.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct CommentThread {
    pub file: String,
    pub line: Option<i32>,
    pub resolved: bool,
    pub comments: Vec<ThreadComment>,
}

/// A single comment within a thread.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ThreadComment {
    pub author: String,
    pub patch_set: Option<i32>,
    pub date: String,
    pub message: String,
}

/// Structured output for JSON format.
#[derive(Debug, Serialize, serde::Deserialize)]
pub struct CommentOutput {
    pub change: ChangeSummary,
    pub review_messages: Vec<ReviewMessage>,
    pub inline_comments: Vec<CommentThread>,
    pub summary: CommentSummaryStats,
}

#[derive(Debug, Serialize, serde::Deserialize)]
pub struct ChangeSummary {
    pub number: i64,
    pub subject: String,
    pub project: String,
    pub branch: String,
    pub status: String,
    pub owner: String,
    pub owner_email: String,
    pub url: String,
}

#[derive(Debug, Serialize, serde::Deserialize)]
pub struct ReviewMessage {
    pub author: String,
    pub patch_set: Option<i32>,
    pub date: String,
    pub message: String,
}

#[derive(Debug, Serialize, serde::Deserialize)]
pub struct CommentSummaryStats {
    pub total_threads: usize,
    pub unresolved: usize,
    pub resolved: usize,
}

/// Build comment threads from a flat map of file -> comments.
pub fn build_threads(comments_by_file: &HashMap<String, Vec<CommentInfo>>) -> Vec<CommentThread> {
    // Collect all comments into a single list with their file paths
    let mut all_comments: Vec<(&str, &CommentInfo)> = Vec::new();
    for (file, comments) in comments_by_file {
        for comment in comments {
            all_comments.push((file, comment));
        }
    }

    // Index by ID for reply chain resolution
    let mut by_id: HashMap<&str, (&str, &CommentInfo)> = HashMap::new();
    for &(file, comment) in &all_comments {
        if let Some(ref id) = comment.id {
            by_id.insert(id.as_str(), (file, comment));
        }
    }

    // Identify root comments (no in_reply_to, or dangling reference)
    let mut roots: Vec<(&str, &CommentInfo)> = Vec::new();
    let mut children: HashMap<&str, Vec<&CommentInfo>> = HashMap::new();

    for &(file, comment) in &all_comments {
        match &comment.in_reply_to {
            Some(parent_id) if by_id.contains_key(parent_id.as_str()) => {
                children
                    .entry(parent_id.as_str())
                    .or_default()
                    .push(comment);
            }
            _ => {
                roots.push((file, comment));
            }
        }
    }

    // Build threads from roots
    let mut threads: Vec<CommentThread> = Vec::new();

    for (file, root) in &roots {
        let mut thread_comments = Vec::new();
        collect_thread(root, &children, &mut thread_comments);

        // Thread is resolved if the last comment has unresolved: false
        let resolved = thread_comments.last().map(|c| !c.2).unwrap_or(false);

        let comments: Vec<ThreadComment> = thread_comments
            .into_iter()
            .map(|(author, ps, _unresolved, date, message)| ThreadComment {
                author,
                patch_set: ps,
                date,
                message,
            })
            .collect();

        threads.push(CommentThread {
            file: file.to_string(),
            line: root.line,
            resolved,
            comments,
        });
    }

    // Sort by file path, then by line number
    threads.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.unwrap_or(0).cmp(&b.line.unwrap_or(0)))
    });

    threads
}

/// Recursively collect comments in a thread, depth-first in chronological order.
fn collect_thread(
    comment: &CommentInfo,
    children: &HashMap<&str, Vec<&CommentInfo>>,
    result: &mut Vec<(String, Option<i32>, bool, String, String)>,
) {
    let author = comment
        .author
        .as_ref()
        .and_then(|a| a.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let unresolved = comment.unresolved.unwrap_or(true);
    let date = comment.updated.clone().unwrap_or_default();
    let message = comment.message.clone().unwrap_or_default();
    let ps = comment.patch_set;

    result.push((author, ps, unresolved, date, message));

    if let Some(id) = &comment.id {
        if let Some(replies) = children.get(id.as_str()) {
            let mut sorted_replies: Vec<&&CommentInfo> = replies.iter().collect();
            sorted_replies.sort_by_key(|c| c.updated.as_deref().unwrap_or(""));
            for reply in sorted_replies {
                collect_thread(reply, children, result);
            }
        }
    }
}

/// Format threads and change info as LLM-friendly text.
pub fn format_text(
    change: &ChangeInfo,
    messages: &[ChangeMessageInfo],
    threads: &[CommentThread],
    gerrit_url: &str,
) -> String {
    let mut out = String::new();

    let number = change.number.unwrap_or(0);
    let subject = change.subject.as_deref().unwrap_or("(no subject)");
    let project = change.project.as_deref().unwrap_or("unknown");
    let branch = change.branch.as_deref().unwrap_or("unknown");
    let status = change.status.as_deref().unwrap_or("UNKNOWN");
    let owner_name = change
        .owner
        .as_ref()
        .and_then(|o| o.name.as_deref())
        .unwrap_or("Unknown");
    let owner_email = change
        .owner
        .as_ref()
        .and_then(|o| o.email.as_deref())
        .unwrap_or("");

    let _ = writeln!(out, "# Change {number} — {subject}");
    let _ = writeln!(
        out,
        "# Project: {project} | Branch: {branch} | Status: {status}"
    );
    if owner_email.is_empty() {
        let _ = writeln!(out, "# Owner: {owner_name}");
    } else {
        let _ = writeln!(out, "# Owner: {owner_name} <{owner_email}>");
    }
    let _ = writeln!(
        out,
        "# URL: {}/c/{}/+/{}",
        gerrit_url.trim_end_matches('/'),
        project,
        number
    );

    // Review messages
    if !messages.is_empty() {
        let _ = writeln!(out, "\n## Review Messages");
        for msg in messages {
            let author = msg
                .author
                .as_ref()
                .and_then(|a| a.name.as_deref())
                .unwrap_or("Unknown");
            let ps = msg
                .revision_number
                .map(|n| format!("Patchset {n}"))
                .unwrap_or_default();
            let date = msg.date.as_deref().unwrap_or("");
            let body = msg.message.as_deref().unwrap_or("");

            let _ = writeln!(out, "\n### {author} ({ps}) — {date}");
            let _ = writeln!(out, "{body}");
        }
    }

    // Inline comments
    if !threads.is_empty() {
        let _ = writeln!(out, "\n## Inline Comments");

        let mut current_file = "";
        for thread in threads {
            if thread.file != current_file {
                current_file = &thread.file;
                let _ = writeln!(out, "\n### File: {current_file}");
            }

            let line_str = thread
                .line
                .map(|l| format!("Line {l}"))
                .unwrap_or_else(|| "File-level".to_string());
            let status = if thread.resolved {
                "RESOLVED"
            } else {
                "UNRESOLVED"
            };
            let count = thread.comments.len();
            let _ = writeln!(
                out,
                "\n#### {line_str} [{status}] ({count} comment{})",
                if count == 1 { "" } else { "s" }
            );

            for c in &thread.comments {
                let ps_str = c.patch_set.map(|n| format!("PS{n}")).unwrap_or_default();
                let _ = writeln!(out, "\n> **{}** ({}) — {}", c.author, ps_str, c.date);
                for line in c.message.lines() {
                    let _ = writeln!(out, "> {line}");
                }
            }
        }
    }

    // Summary
    let total = threads.len();
    let unresolved = threads.iter().filter(|t| !t.resolved).count();
    let resolved = threads.iter().filter(|t| t.resolved).count();

    let _ = writeln!(out, "\n## Summary");
    let _ = writeln!(out, "- Total inline comment threads: {total}");
    let _ = writeln!(out, "- Unresolved: {unresolved}");
    let _ = writeln!(out, "- Resolved: {resolved}");

    out
}

/// Format threads and change info as structured JSON.
pub fn format_json(
    change: &ChangeInfo,
    messages: &[ChangeMessageInfo],
    threads: &[CommentThread],
    gerrit_url: &str,
) -> serde_json::Value {
    let number = change.number.unwrap_or(0);
    let project = change.project.as_deref().unwrap_or("unknown");

    let review_messages: Vec<ReviewMessage> = messages
        .iter()
        .map(|m| ReviewMessage {
            author: m
                .author
                .as_ref()
                .and_then(|a| a.name.clone())
                .unwrap_or_else(|| "Unknown".to_string()),
            patch_set: m.revision_number,
            date: m.date.clone().unwrap_or_default(),
            message: m.message.clone().unwrap_or_default(),
        })
        .collect();

    let total = threads.len();
    let unresolved = threads.iter().filter(|t| !t.resolved).count();
    let resolved = threads.iter().filter(|t| t.resolved).count();

    let output = CommentOutput {
        change: ChangeSummary {
            number,
            subject: change.subject.clone().unwrap_or_default(),
            project: project.to_string(),
            branch: change.branch.clone().unwrap_or_default(),
            status: change.status.clone().unwrap_or_default(),
            owner: change
                .owner
                .as_ref()
                .and_then(|o| o.name.clone())
                .unwrap_or_default(),
            owner_email: change
                .owner
                .as_ref()
                .and_then(|o| o.email.clone())
                .unwrap_or_default(),
            url: format!(
                "{}/c/{}/+/{}",
                gerrit_url.trim_end_matches('/'),
                project,
                number
            ),
        },
        review_messages,
        inline_comments: threads.to_vec(),
        summary: CommentSummaryStats {
            total_threads: total,
            unresolved,
            resolved,
        },
    };

    serde_json::to_value(output).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gerrit::{AccountInfo, CommentInfo};

    fn comment(id: &str, file: &str) -> CommentBuilder {
        CommentBuilder {
            id: id.to_string(),
            file: file.to_string(),
            line: Some(1),
            author: "Author".to_string(),
            message: "Comment".to_string(),
            reply_to: None,
            unresolved: Some(true),
            ps: Some(1),
        }
    }

    struct CommentBuilder {
        id: String,
        file: String,
        line: Option<i32>,
        author: String,
        message: String,
        reply_to: Option<String>,
        unresolved: Option<bool>,
        ps: Option<i32>,
    }

    impl CommentBuilder {
        fn line(mut self, l: i32) -> Self {
            self.line = Some(l);
            self
        }
        fn no_line(mut self) -> Self {
            self.line = None;
            self
        }
        fn author(mut self, a: &str) -> Self {
            self.author = a.to_string();
            self
        }
        fn message(mut self, m: &str) -> Self {
            self.message = m.to_string();
            self
        }
        fn reply_to(mut self, r: &str) -> Self {
            self.reply_to = Some(r.to_string());
            self
        }
        fn unresolved(mut self, u: bool) -> Self {
            self.unresolved = Some(u);
            self
        }
        fn no_unresolved(mut self) -> Self {
            self.unresolved = None;
            self
        }
        fn ps(mut self, p: i32) -> Self {
            self.ps = Some(p);
            self
        }

        fn build(self) -> (String, CommentInfo) {
            (
                self.file.clone(),
                CommentInfo {
                    id: Some(self.id),
                    path: Some(self.file),
                    line: self.line,
                    range: None,
                    in_reply_to: self.reply_to,
                    message: Some(self.message),
                    updated: Some("2025-02-10 14:00:00".to_string()),
                    author: Some(AccountInfo {
                        account_id: 1,
                        name: Some(self.author),
                        email: None,
                        username: None,
                        display_name: None,
                    }),
                    patch_set: self.ps,
                    unresolved: self.unresolved,
                },
            )
        }
    }

    fn comments_map(items: Vec<(String, CommentInfo)>) -> HashMap<String, Vec<CommentInfo>> {
        let mut map: HashMap<String, Vec<CommentInfo>> = HashMap::new();
        for (file, comment) in items {
            map.entry(file).or_default().push(comment);
        }
        map
    }

    #[test]
    fn build_threads_single_comment() {
        let items = vec![comment("c1", "src/main.rs")
            .line(10)
            .author("Bob")
            .message("Fix this")
            .build()];
        let threads = build_threads(&comments_map(items));
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].comments.len(), 1);
        assert_eq!(threads[0].file, "src/main.rs");
    }

    #[test]
    fn build_threads_reply_chain() {
        let items = vec![
            comment("c1", "src/main.rs")
                .line(10)
                .author("Bob")
                .message("Fix this")
                .build(),
            comment("c2", "src/main.rs")
                .line(10)
                .author("Alice")
                .message("Done")
                .reply_to("c1")
                .unresolved(false)
                .ps(2)
                .build(),
        ];
        let threads = build_threads(&comments_map(items));
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].comments.len(), 2);
    }

    #[test]
    fn build_threads_multiple_files() {
        let items = vec![
            comment("c1", "src/a.rs")
                .line(1)
                .author("Bob")
                .message("Comment A")
                .build(),
            comment("c2", "src/b.rs")
                .line(2)
                .author("Bob")
                .message("Comment B")
                .build(),
            comment("c3", "src/c.rs")
                .line(3)
                .author("Bob")
                .message("Comment C")
                .build(),
        ];
        let threads = build_threads(&comments_map(items));
        assert_eq!(threads.len(), 3);
    }

    #[test]
    fn build_threads_deep_chain() {
        let items = vec![
            comment("c1", "f.rs").author("A").message("1").build(),
            comment("c2", "f.rs")
                .author("B")
                .message("2")
                .reply_to("c1")
                .build(),
            comment("c3", "f.rs")
                .author("A")
                .message("3")
                .reply_to("c2")
                .build(),
            comment("c4", "f.rs")
                .author("B")
                .message("4")
                .reply_to("c3")
                .unresolved(false)
                .ps(2)
                .build(),
        ];
        let threads = build_threads(&comments_map(items));
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].comments.len(), 4);
    }

    #[test]
    fn build_threads_resolved() {
        let items = vec![
            comment("c1", "f.rs").author("A").message("Fix").build(),
            comment("c2", "f.rs")
                .author("B")
                .message("Done")
                .reply_to("c1")
                .unresolved(false)
                .ps(2)
                .build(),
        ];
        let threads = build_threads(&comments_map(items));
        assert!(threads[0].resolved);
    }

    #[test]
    fn build_threads_unresolved() {
        let items = vec![
            comment("c1", "f.rs").author("A").message("Fix").build(),
            comment("c2", "f.rs")
                .author("B")
                .message("Why?")
                .reply_to("c1")
                .ps(2)
                .build(),
        ];
        let threads = build_threads(&comments_map(items));
        assert!(!threads[0].resolved);
    }

    #[test]
    fn build_threads_no_unresolved_field() {
        let items = vec![comment("c1", "f.rs")
            .author("A")
            .message("Comment")
            .no_unresolved()
            .build()];
        let threads = build_threads(&comments_map(items));
        // Without unresolved field, treated as unresolved
        assert!(!threads[0].resolved);
    }

    #[test]
    fn build_threads_sorted_by_line() {
        let items = vec![
            comment("c1", "f.rs")
                .line(50)
                .author("A")
                .message("Late")
                .build(),
            comment("c2", "f.rs")
                .line(10)
                .author("A")
                .message("Early")
                .build(),
        ];
        let threads = build_threads(&comments_map(items));
        assert_eq!(threads[0].line, Some(10));
        assert_eq!(threads[1].line, Some(50));
    }

    #[test]
    fn build_threads_dangling_reply() {
        let items = vec![comment("c1", "f.rs")
            .author("A")
            .message("Reply to missing")
            .reply_to("nonexistent")
            .build()];
        let threads = build_threads(&comments_map(items));
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].comments.len(), 1);
    }

    #[test]
    fn build_threads_file_level_comment() {
        let items = vec![comment("c1", "f.rs")
            .no_line()
            .author("A")
            .message("File comment")
            .build()];
        let threads = build_threads(&comments_map(items));
        assert_eq!(threads[0].line, None);
    }

    fn test_change(number: i64) -> ChangeInfo {
        ChangeInfo {
            id: None,
            project: Some("proj".into()),
            branch: Some("main".into()),
            change_id: None,
            subject: Some("Test".into()),
            status: Some("NEW".into()),
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

    #[test]
    fn format_text_basic() {
        let items = vec![comment("c1", "src/main.rs")
            .line(42)
            .author("Bob")
            .message("Fix this")
            .ps(3)
            .build()];
        let threads = build_threads(&comments_map(items));

        let change = ChangeInfo {
            project: Some("myproject".into()),
            change_id: Some("Iabcdef".into()),
            subject: Some("Fix bug".into()),
            number: Some(12345),
            owner: Some(AccountInfo {
                account_id: 1,
                name: Some("Alice".into()),
                email: Some("alice@example.com".into()),
                username: None,
                display_name: None,
            }),
            ..test_change(12345)
        };

        let text = format_text(&change, &[], &threads, "https://review.example.com");
        assert!(text.contains("# Change 12345"));
        assert!(text.contains("UNRESOLVED"));
        assert!(text.contains("Bob"));
        assert!(text.contains("Fix this"));
    }

    #[test]
    fn format_text_unresolved_only() {
        let items = vec![
            comment("c1", "f.rs")
                .line(1)
                .author("A")
                .message("Resolved")
                .unresolved(false)
                .build(),
            comment("c2", "f.rs")
                .line(10)
                .author("A")
                .message("Unresolved")
                .build(),
        ];
        let threads = build_threads(&comments_map(items));
        let unresolved = threads.iter().filter(|t| !t.resolved).count();
        assert_eq!(unresolved, 1);
    }

    #[test]
    fn format_text_no_comments() {
        let change = test_change(1);
        let text = format_text(&change, &[], &[], "https://review.example.com");
        assert!(text.contains("Total inline comment threads: 0"));
    }

    #[test]
    fn format_json_roundtrip() {
        let items = vec![comment("c1", "f.rs").build()];
        let threads = build_threads(&comments_map(items));
        let change = test_change(1);
        let json = format_json(&change, &[], &threads, "https://review.example.com");
        let output: CommentOutput = serde_json::from_value(json).unwrap();
        assert_eq!(output.summary.total_threads, 1);
    }

    #[test]
    fn format_json_schema() {
        let change = test_change(1);
        let json = format_json(&change, &[], &[], "https://review.example.com");
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("change"));
        assert!(obj.contains_key("review_messages"));
        assert!(obj.contains_key("inline_comments"));
        assert!(obj.contains_key("summary"));
    }
}
