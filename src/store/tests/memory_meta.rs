#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for the small `memories`-table reads added to
//! `store::memory_meta` (`ids_matching_kind`, `kind_and_body`,
//! `rank_signals`, `keeper_stats`) — real rows via `memory_row::insert`, no
//! mocks.

use comemory::memory::{Frontmatter, Kind, References, Relations};
use comemory::store::memory_meta::{
    fetch_extra, ids_matching_kind, keeper_stats, kind_and_body, rank_signals,
};
use comemory::store::{connection, memory_row};
use rusqlite::Connection;
use time::OffsetDateTime;

/// Insert one memory row with an explicit id/kind/repo via the production
/// writer, so the mirror row is byte-identical to a real save.
fn seed(conn: &Connection, id: &str, kind: Kind, repo: &str) {
    let fm = Frontmatter {
        id: id.to_string(),
        kind,
        repo: repo.to_string(),
        tags: Vec::new(),
        author: "alice".to_string(),
        created: OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("valid timestamp"),
        quality: 3,
        schema: 1,
        content_hash: format!("hash-{id}"),
        references: References::default(),
        relations: Relations::default(),
    };
    memory_row::insert(
        conn,
        &fm,
        "body text",
        "note",
        &format!("/data/.comemory/memories/{id}-note.md"),
        &[],
    )
    .expect("insert memory");
}

fn seed_db() -> Connection {
    let dir = tempfile::tempdir().expect("tempdir");
    connection::open(dir.path().join("comemory.db")).expect("open")
}

#[test]
fn ids_matching_kind_filters_and_preserves_membership_only() {
    let conn = seed_db();
    seed(&conn, "aaaa0001", Kind::Decision, "r");
    seed(&conn, "aaaa0002", Kind::Bug, "r");
    seed(&conn, "aaaa0003", Kind::Decision, "r");

    let ids = ["aaaa0001", "aaaa0002", "aaaa0003", "aaaa9999"];
    let matched = ids_matching_kind(&conn, "decision", &ids).expect("query");
    let mut sorted = matched.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["aaaa0001".to_string(), "aaaa0003".to_string()]);
}

#[test]
fn ids_matching_kind_with_empty_ids_is_empty() {
    let conn = seed_db();
    let matched = ids_matching_kind(&conn, "decision", &[]).expect("query");
    assert!(matched.is_empty());
}

#[test]
fn kind_and_body_returns_none_for_missing_or_soft_deleted() {
    let conn = seed_db();
    seed(&conn, "aaaa0001", Kind::Note, "r");
    conn.execute(
        "UPDATE memories SET deleted_at = '2026-01-01T00:00:00Z' WHERE id = 'aaaa0001'",
        [],
    )
    .expect("soft delete");

    assert!(kind_and_body(&conn, "aaaa0001").expect("query").is_none());
    assert!(kind_and_body(&conn, "missing").expect("query").is_none());
}

#[test]
fn kind_and_body_returns_live_row() {
    let conn = seed_db();
    seed(&conn, "aaaa0001", Kind::Convention, "r");
    let (kind, body) = kind_and_body(&conn, "aaaa0001")
        .expect("query")
        .expect("row present");
    assert_eq!(kind, "convention");
    assert_eq!(body, "body text");
}

#[test]
fn rank_signals_none_for_missing_row_and_neutral_feedback_when_absent() {
    let conn = seed_db();
    seed(&conn, "aaaa0001", Kind::Decision, "r");

    assert!(rank_signals(&conn, "missing").expect("query").is_none());

    let sig = rank_signals(&conn, "aaaa0001")
        .expect("query")
        .expect("row present");
    assert_eq!(sig.quality, 3);
    assert_eq!(sig.used, 0);
    assert_eq!(sig.irrelevant, 0);
    assert_eq!(sig.body, "body text");
}

#[test]
fn rank_signals_joins_feedback_counters() {
    let conn = seed_db();
    seed(&conn, "aaaa0001", Kind::Decision, "r");
    conn.execute(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count) VALUES ('aaaa0001', 4, 1)",
        [],
    )
    .expect("seed feedback");

    let sig = rank_signals(&conn, "aaaa0001")
        .expect("query")
        .expect("row present");
    assert_eq!(sig.used, 4);
    assert_eq!(sig.irrelevant, 1);
}

#[test]
fn keeper_stats_batches_exactly_the_requested_ids() {
    let conn = seed_db();
    seed(&conn, "aaaa0001", Kind::Decision, "r1");
    seed(&conn, "aaaa0002", Kind::Bug, "r2");
    seed(&conn, "aaaa0003", Kind::Note, "r3");

    let ids = ["aaaa0001", "aaaa0003"];
    let rows = keeper_stats(&conn, &ids).expect("query");
    let mut ids_seen: Vec<&str> = rows.iter().map(|(id, _)| id.as_str()).collect();
    ids_seen.sort_unstable();
    assert_eq!(ids_seen, vec!["aaaa0001", "aaaa0003"]);

    let by_id: std::collections::HashMap<_, _> = rows.into_iter().collect();
    assert_eq!(by_id["aaaa0001"].repo.as_deref(), Some("r1"));
    assert_eq!(by_id["aaaa0003"].kind, "note");
}

#[test]
fn keeper_stats_with_empty_ids_is_empty() {
    let conn = seed_db();
    let rows = keeper_stats(&conn, &[]).expect("query");
    assert!(rows.is_empty());
}

#[test]
fn fetch_extra_is_none_for_missing_or_soft_deleted() {
    let conn = seed_db();
    seed(&conn, "aaaa0001", Kind::Note, "r");
    conn.execute(
        "UPDATE memories SET deleted_at = '2026-01-01T00:00:00Z' WHERE id = 'aaaa0001'",
        [],
    )
    .expect("soft delete");

    assert!(fetch_extra(&conn, "aaaa0001").expect("query").is_none());
    assert!(fetch_extra(&conn, "missing").expect("query").is_none());
}

#[test]
fn fetch_extra_returns_the_live_row() {
    let conn = seed_db();
    seed(&conn, "aaaa0001", Kind::Decision, "r");

    let extra = fetch_extra(&conn, "aaaa0001")
        .expect("query")
        .expect("row present");
    assert_eq!(extra.body, "body text");
    assert_eq!(extra.quality, 3);
    assert_eq!(extra.access_count, 0);
    assert!(extra.last_accessed.is_none());
    assert!(!extra.created.is_empty());
    assert!(!extra.updated.is_empty());
}
