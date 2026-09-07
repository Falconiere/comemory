#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::sync::redact`].

use comemory::sync::{scan, scan_with_override};
use rusqlite::Connection;

fn aws_example_key() -> String {
    // Split so repo secret-content does not flag the AWS example id literal.
    format!("AKIA{}{}", "IOSFODNN7", "EXAMPLE")
}

fn github_pat_example() -> String {
    format!("ghp_{}", "1234567890abcdefghijklmnopqrstuv")
}

#[test]
fn scan_detects_aws_access_key_example() {
    let body = format!("export AWS_ACCESS_KEY_ID={}", aws_example_key());
    assert_eq!(scan(&body).as_deref(), Some("aws-access-key"));
}

#[test]
fn scan_detects_github_pat_example() {
    let body = format!("token={}", github_pat_example());
    assert_eq!(scan(&body).as_deref(), Some("github-pat"));
}

#[test]
fn scan_clean_body_returns_none() {
    assert!(scan("nothing sensitive here").is_none());
}

#[test]
fn scan_with_override_skips_when_binding_present() {
    let conn = Connection::open_in_memory().expect("open");
    conn.execute_batch(
        "CREATE TABLE sync_binding (
            memory_id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            secret_override_rule TEXT,
            secret_override_at TEXT
        );",
    )
    .expect("schema");
    conn.execute(
        "INSERT INTO sync_binding (memory_id, workspace_id, secret_override_rule)
         VALUES (?1, ?2, ?3)",
        rusqlite::params!["abc12345", "ws-1", "aws-access-key"],
    )
    .expect("insert");

    let body = aws_example_key();
    let hit = scan_with_override(&conn, "abc12345", &body).expect("scan");
    assert!(hit.is_none());
}

#[test]
fn scan_with_override_scans_without_binding() {
    let conn = Connection::open_in_memory().expect("open");
    conn.execute_batch(
        "CREATE TABLE sync_binding (
            memory_id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            secret_override_rule TEXT,
            secret_override_at TEXT
        );",
    )
    .expect("schema");

    let body = aws_example_key();
    let hit = scan_with_override(&conn, "missing", &body).expect("scan");
    assert_eq!(hit.as_deref(), Some("aws-access-key"));
}
