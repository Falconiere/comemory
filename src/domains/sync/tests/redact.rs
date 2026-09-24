#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::domains::sync::redact`] — the per-memory override, and
//! the re-export every existing caller imports through. The shared rule set's
//! own behavior is tested beside it in `src/utilities/tests/secret_scan.rs`.

use comemory::domains::sync::redact::{scan, scan_with_override};
use comemory::domains::sync::{
    scan as scan_reexport, scan_with_override as scan_override_reexport,
};
use rusqlite::Connection;

fn aws_example_key() -> String {
    // Split so repo secret-content does not flag the AWS example id literal.
    format!("AKIA{}{}", "IOSFODNN7", "EXAMPLE")
}

/// The rule set moved to `utilities::secret_scan`; both paths callers still
/// import through must answer from it.
#[test]
fn the_re_exported_scan_still_answers_from_the_shared_rule_set() {
    let body = format!("export AWS_ACCESS_KEY_ID={}", aws_example_key());
    assert_eq!(scan(&body).as_deref(), Some("aws-access-key-id"));
    assert_eq!(scan_reexport(&body).as_deref(), Some("aws-access-key-id"));
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
