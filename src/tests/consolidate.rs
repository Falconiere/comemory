#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end cover for `comemory::consolidate::detect` — scan → cluster →
//! keeper → resolved filter over a store written by the production row path.

use comemory::config::Config;
use comemory::consolidate::{Options, Scan, detect};
use comemory::memory::{Frontmatter, Kind, References, Relations};
use time::OffsetDateTime;

/// Default options at the shipped radius, corpus-wide, resolved hidden.
fn opts() -> Options {
    Options {
        radius: Config::defaults().rank.near_dup_hamming,
        repo: None,
        include_resolved: false,
    }
}

/// Mirror `body` into the store through the production `memory_row::insert`,
/// so the row carries the same simhash a real save writes.
fn save(conn: &rusqlite::Connection, id: &str, body: &str, repo: &str) -> String {
    let fm = Frontmatter {
        id: id.to_string(),
        kind: Kind::Note,
        repo: repo.to_string(),
        tags: Vec::new(),
        author: "f".to_string(),
        created: OffsetDateTime::now_utc(),
        quality: 3,
        schema: 1,
        content_hash: id.to_string(),
        references: References::default(),
        relations: Relations::default(),
    };
    let md_path = format!("memories/{id}.md");
    comemory::store::memory_row::insert(conn, &fm, body, id, &md_path, &fm.tags)
        .expect("mirror into the store");
    fm.id
}

/// A temp data dir plus its open store.
fn store() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// Cluster membership, ids only, in report order.
fn ids(scan: &Scan) -> Vec<Vec<String>> {
    scan.clusters
        .iter()
        .map(|c| c.members.iter().map(|m| m.id.clone()).collect())
        .collect()
}

#[test]
fn near_duplicates_cluster_and_the_unrelated_memory_stays_out() {
    let (_d, conn) = store();
    save(
        &conn,
        "aaaa0001",
        "postgres advisory lock ordering fix for the migration runner",
        "demo",
    );
    save(
        &conn,
        "aaaa0002",
        "postgres advisory lock ordering fix for the migration worker",
        "demo",
    );
    save(
        &conn,
        "aaaa0003",
        "postgres advisory lock ordering fix for the migration daemon",
        "demo",
    );
    save(
        &conn,
        "bbbb0001",
        "terraform state locking through a dynamodb table",
        "demo",
    );

    let scan = detect(&conn, &opts()).expect("detect");
    assert_eq!(
        ids(&scan),
        vec![vec![
            "aaaa0001".to_string(),
            "aaaa0002".to_string(),
            "aaaa0003".to_string()
        ]],
        "one cluster holding the three near-duplicates, unrelated topic left out"
    );
    assert_eq!(scan.scanned, 4, "every live row was compared");
    assert_eq!(scan.skipped_unhashed, 0);
    assert_eq!(scan.clustered(), 3);
}

#[test]
fn the_repo_filter_isolates_the_scan() {
    let (_d, conn) = store();
    save(
        &conn,
        "aaaa0001",
        "kubernetes ingress certificate renewal job",
        "alpha",
    );
    save(
        &conn,
        "bbbb0001",
        "kubernetes ingress certificate renewal task",
        "beta",
    );

    let scan = detect(
        &conn,
        &Options {
            repo: Some("alpha".into()),
            ..opts()
        },
    )
    .expect("detect");
    assert!(
        scan.clusters.is_empty(),
        "the twin lives in another repo, so nothing clusters"
    );
    assert_eq!(scan.scanned, 1, "only the alpha row was compared");
}

#[test]
fn a_resolved_cluster_is_hidden_by_default_and_returned_with_include_resolved() {
    let (_d, conn) = store();
    save(
        &conn,
        "aaaa0001",
        "redis cache eviction policy tuning for the session store",
        "demo",
    );
    save(
        &conn,
        "bbbb0001",
        "redis cache eviction policy tuning for the session cache",
        "demo",
    );
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES('memory','bbbb0001','memory','aaaa0001','supersedes','2026-07-28T00:00:00Z')",
        [],
    )
    .expect("insert supersedes edge");

    assert!(
        detect(&conn, &opts()).expect("detect").clusters.is_empty(),
        "settled duplication is not re-reported"
    );

    let shown = detect(
        &conn,
        &Options {
            include_resolved: true,
            ..opts()
        },
    )
    .expect("detect");
    assert_eq!(shown.clusters.len(), 1);
    assert!(shown.clusters[0].resolved, "and it is marked as resolved");
}

#[test]
fn an_empty_corpus_and_a_clean_corpus_both_report_nothing() {
    let (_d, conn) = store();
    let empty = detect(&conn, &opts()).expect("detect");
    assert!(empty.clusters.is_empty());
    assert_eq!(empty.scanned, 0);

    save(
        &conn,
        "aaaa0001",
        "graphql federation gateway timeout budget",
        "demo",
    );
    save(
        &conn,
        "bbbb0001",
        "docker compose volume mount permissions",
        "demo",
    );
    let clean = detect(&conn, &opts()).expect("detect");
    assert!(
        clean.clusters.is_empty(),
        "distinct memories are not near-duplicates"
    );
    assert_eq!(clean.scanned, 2);
}

#[test]
fn detect_is_deterministic_across_runs() {
    let (_d, conn) = store();
    save(
        &conn,
        "aaaa0001",
        "sqlite wal checkpoint starvation under long readers",
        "demo",
    );
    save(
        &conn,
        "bbbb0001",
        "sqlite wal checkpoint starvation under slow readers",
        "demo",
    );
    assert_eq!(
        ids(&detect(&conn, &opts()).expect("detect")),
        ids(&detect(&conn, &opts()).expect("detect")),
        "two passes over one corpus must agree"
    );
}
