// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use grt::gerrit::{Credentials, GerritClient};
use url::Url;

fn test_client(server_url: &str) -> GerritClient {
    let url = Url::parse(server_url).unwrap();
    GerritClient::new(url, None).unwrap()
}

fn test_client_with_auth(server_url: &str) -> GerritClient {
    let url = Url::parse(server_url).unwrap();
    let creds = Credentials {
        username: "testuser".into(),
        password: "testpass".into(),
    };
    GerritClient::new(url, Some(creds)).unwrap()
}

#[tokio::test]
async fn get_version_success() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/config/server/version")
        .with_status(200)
        .with_body(")]}'\n\"3.9.1\"")
        .create_async()
        .await;

    let client = test_client(&server.url());
    let version = client.get_version().await.unwrap();
    assert_eq!(version, "3.9.1");
    mock.assert_async().await;
}

#[tokio::test]
async fn get_version_connection_error() {
    // Use a URL that will fail to connect
    let client = test_client("http://127.0.0.1:1");
    let result = client.get_version().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn get_self_account_success() {
    let mut server = mockito::Server::new_async().await;
    let body = r#")]}'
{
  "_account_id": 1000096,
  "name": "Alice Smith",
  "email": "alice@example.com",
  "username": "alice"
}"#;
    let mock = server
        .mock("GET", "/a/accounts/self")
        .match_header("authorization", mockito::Matcher::Any)
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = test_client_with_auth(&server.url());
    let account = client.get_self_account().await.unwrap();
    assert_eq!(account.account_id, 1000096);
    assert_eq!(account.name.as_deref(), Some("Alice Smith"));
    mock.assert_async().await;
}

#[tokio::test]
async fn get_self_account_401() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/accounts/self")
        .with_status(401)
        .with_body("Authentication required")
        .create_async()
        .await;

    let client = test_client(&server.url());
    let result = client.get_self_account().await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("401"),
        "error should mention 401: {err_msg}"
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn query_changes_success() {
    let mut server = mockito::Server::new_async().await;
    let body = r#")]}'
[
  {
    "id": "proj~main~Iabcdef",
    "project": "proj",
    "branch": "main",
    "change_id": "Iabcdef",
    "subject": "Test change",
    "status": "NEW",
    "_number": 12345,
    "owner": { "_account_id": 1000096, "name": "Alice" }
  }
]"#;
    let mock = server
        .mock(
            "GET",
            mockito::Matcher::Regex(r"/changes/\?q=.*".to_string()),
        )
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = test_client(&server.url());
    let changes = client.query_changes("status:open").await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].number, Some(12345));
    mock.assert_async().await;
}

#[tokio::test]
async fn query_changes_empty() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock(
            "GET",
            mockito::Matcher::Regex(r"/changes/\?q=.*".to_string()),
        )
        .with_status(200)
        .with_body(")]}'\n[]")
        .create_async()
        .await;

    let client = test_client(&server.url());
    let changes = client.query_changes("status:abandoned").await.unwrap();
    assert!(changes.is_empty());
    mock.assert_async().await;
}

#[tokio::test]
async fn get_change_all_revisions_success() {
    let mut server = mockito::Server::new_async().await;
    let body = r#")]}'
{
  "id": "proj~main~Iabcdef",
  "project": "proj",
  "branch": "main",
  "change_id": "Iabcdef",
  "subject": "Fix bug",
  "status": "NEW",
  "topic": "my-feature",
  "_number": 12345,
  "owner": { "_account_id": 1000096, "name": "Alice", "username": "alice" },
  "current_revision": "def456",
  "revisions": {
    "abc123": {
      "_number": 1,
      "ref": "refs/changes/45/12345/1",
      "commit": { "subject": "Fix bug v1" }
    },
    "def456": {
      "_number": 2,
      "ref": "refs/changes/45/12345/2",
      "commit": { "subject": "Fix bug v2" }
    }
  }
}"#;
    let mock = server
        .mock(
            "GET",
            mockito::Matcher::Regex(r"/changes/12345/detail\?o=ALL_REVISIONS.*".to_string()),
        )
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = test_client(&server.url());
    let change = client.get_change_all_revisions("12345").await.unwrap();
    assert_eq!(change.number, Some(12345));
    assert_eq!(change.topic.as_deref(), Some("my-feature"));
    let revisions = change.revisions.unwrap();
    assert_eq!(revisions.len(), 2);
    assert!(revisions.contains_key("abc123"));
    assert!(revisions.contains_key("def456"));
    assert_eq!(revisions["abc123"].number, Some(1));
    assert_eq!(revisions["def456"].number, Some(2));
    assert_eq!(
        revisions["abc123"].git_ref.as_deref(),
        Some("refs/changes/45/12345/1")
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn get_change_comments_success() {
    let mut server = mockito::Server::new_async().await;
    let body = r#")]}'
{
  "src/main.rs": [
    {
      "id": "c1",
      "path": "src/main.rs",
      "line": 10,
      "message": "Fix this",
      "author": { "_account_id": 1, "name": "Bob" },
      "patch_set": 1,
      "unresolved": true
    }
  ]
}"#;
    let mock = server
        .mock("GET", "/changes/12345/comments")
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = test_client(&server.url());
    let comments = client.get_change_comments("12345").await.unwrap();
    assert!(comments.contains_key("src/main.rs"));
    assert_eq!(comments["src/main.rs"].len(), 1);
    mock.assert_async().await;
}

#[tokio::test]
async fn get_change_comments_empty() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/changes/12345/comments")
        .with_status(200)
        .with_body(")]}'\n{}")
        .create_async()
        .await;

    let client = test_client(&server.url());
    let comments = client.get_change_comments("12345").await.unwrap();
    assert!(comments.is_empty());
    mock.assert_async().await;
}
