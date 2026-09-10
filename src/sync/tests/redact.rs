#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::sync::redact`].

use comemory::sync::redact::{RULE_SET_VERSION, findings, redact, scan, scan_with_override};
use comemory::sync::{scan as scan_reexport, scan_with_override as scan_override_reexport};
use rusqlite::Connection;

fn aws_example_key() -> String {
    // Split so repo secret-content does not flag the AWS example id literal.
    format!("AKIA{}{}", "IOSFODNN7", "EXAMPLE")
}

fn github_pat_example() -> String {
    format!("ghp_{}", "1234567890abcdefghijklmnopqrstuv")
}

#[test]
fn rule_set_version_is_one() {
    assert_eq!(RULE_SET_VERSION, 1);
}

#[test]
fn scan_detects_aws_access_key_example() {
    let body = format!("export AWS_ACCESS_KEY_ID={}", aws_example_key());
    assert_eq!(scan(&body).as_deref(), Some("aws-access-key-id"));
    assert_eq!(scan_reexport(&body).as_deref(), Some("aws-access-key-id"));
}

#[test]
fn scan_detects_github_pat_example() {
    let body = format!("token={}", github_pat_example());
    assert_eq!(scan(&body).as_deref(), Some("github-token"));
}

#[test]
fn scan_clean_body_returns_none() {
    assert!(scan("nothing sensitive here").is_none());
}

#[test]
fn findings_aggregate_counts() {
    let body = format!(
        "a={} b={} c={}",
        aws_example_key(),
        aws_example_key(),
        github_pat_example()
    );
    let f = findings(&body);
    let aws = f.iter().find(|x| x.rule == "aws-access-key-id").unwrap();
    assert_eq!(aws.count, 2);
    let gh = f.iter().find(|x| x.rule == "github-token").unwrap();
    assert_eq!(gh.count, 1);
}

#[test]
fn redact_strips_secret_and_attests() {
    let key = aws_example_key();
    let body = format!("key={key} ok");
    let (out, f) = redact(&body);
    assert!(!out.contains(&key));
    assert!(out.contains("[REDACTED:aws-access-key-id]"));
    let aws = f.iter().find(|x| x.rule == "aws-access-key-id").unwrap();
    assert_eq!(aws.count, 1);
}

#[test]
fn redact_clean_body_has_empty_findings() {
    let (out, f) = redact("nothing sensitive here");
    assert_eq!(out, "nothing sensitive here");
    assert!(f.is_empty());
}

#[test]
fn redact_multiple_rules() {
    let body = format!("a={} b={}", aws_example_key(), github_pat_example());
    let (out, f) = redact(&body);
    assert!(out.contains("[REDACTED:aws-access-key-id]"));
    assert!(out.contains("[REDACTED:github-token]"));
    assert!(!out.contains(&aws_example_key()));
    assert!(!out.contains(&github_pat_example()));
    assert_eq!(f.len(), 2);
    assert_eq!(
        f.iter()
            .find(|x| x.rule == "aws-access-key-id")
            .unwrap()
            .count,
        1
    );
    assert_eq!(
        f.iter().find(|x| x.rule == "github-token").unwrap().count,
        1
    );
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
        rusqlite::params!["abc12345", "ws-1", "aws-access-key-id"],
    )
    .expect("insert");

    let body = aws_example_key();
    let hit = scan_with_override(&conn, "abc12345", &body).expect("scan");
    assert!(hit.is_none());
    assert!(
        scan_override_reexport(&conn, "abc12345", &body)
            .unwrap()
            .is_none()
    );
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
    assert_eq!(hit.as_deref(), Some("aws-access-key-id"));
}
