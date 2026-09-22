#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`comemory::store::remote_code`] against a real migrated database — a
//! pulled projection is written and read per generation, and replacing one
//! generation leaves every other generation alone.

use comemory::store::remote_code::{self, Edge, File, Projection, Symbol};
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

const REPO: &str = "Falconiere/comemory";

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

/// A projection shaped like a real one: two files, their symbols, an import
/// edge between them and a mined co-change pair.
fn projection(weight: i64) -> Projection {
    Projection {
        files: vec![
            File {
                path: "src/lib.rs".to_string(),
                blob_oid: "aaaa1111".to_string(),
            },
            File {
                path: "src/store.rs".to_string(),
                blob_oid: "bbbb2222".to_string(),
            },
        ],
        symbols: vec![Symbol {
            path: "src/lib.rs".to_string(),
            symbol: "run".to_string(),
            kind: "function".to_string(),
            lang: "rust".to_string(),
            line_start: 10,
            line_end: 42,
        }],
        edges: vec![
            Edge {
                rel: "imports".to_string(),
                src_path: "src/lib.rs".to_string(),
                dst_path: "src/store.rs".to_string(),
                weight: 1,
                anchor: Some("aaaa1111".to_string()),
            },
            Edge {
                rel: "co_changed".to_string(),
                src_path: "src/lib.rs".to_string(),
                dst_path: "src/store.rs".to_string(),
                weight,
                anchor: Some("commit-7".to_string()),
            },
        ],
    }
}

#[test]
fn a_projection_round_trips_per_generation() {
    let (_dir, conn) = migrated_db();
    remote_code::replace_generation(&conn, REPO, "gen1", &projection(3)).expect("replace");

    assert_eq!(
        remote_code::files(&conn, REPO, "gen1").expect("files"),
        projection(3).files
    );
    assert_eq!(
        remote_code::symbols(&conn, REPO, "gen1").expect("symbols"),
        projection(3).symbols
    );
    let edges = remote_code::edges(&conn, REPO, "gen1").expect("edges");
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0].rel, "co_changed", "ascending by relation");
    assert_eq!(edges[0].weight, 3);
    assert_eq!(edges[0].anchor.as_deref(), Some("commit-7"));
}

#[test]
fn replacing_a_generation_replaces_its_weights_rather_than_accumulating_them() {
    let (_dir, conn) = migrated_db();
    remote_code::replace_generation(&conn, REPO, "gen1", &projection(3)).expect("first");

    // The same generation applied again — a replay. Weights must not double.
    remote_code::replace_generation(&conn, REPO, "gen1", &projection(3)).expect("replay");

    let edges = remote_code::edges(&conn, REPO, "gen1").expect("edges");
    assert_eq!(edges.len(), 2, "a replay writes the same rows, not more");
    assert_eq!(
        edges[0].weight, 3,
        "the weight is the generation's, not a running total"
    );
}

#[test]
fn replacing_one_generation_leaves_every_other_generation_alone() {
    let (_dir, conn) = migrated_db();
    remote_code::replace_generation(&conn, REPO, "gen1", &projection(3)).expect("gen1");
    remote_code::replace_generation(&conn, REPO, "gen2", &projection(9)).expect("gen2");

    remote_code::replace_generation(&conn, REPO, "gen2", &Projection::default()).expect("empty");

    assert_eq!(
        remote_code::files(&conn, REPO, "gen1")
            .expect("files")
            .len(),
        2,
        "gen1 is untouched"
    );
    assert!(
        remote_code::files(&conn, REPO, "gen2")
            .expect("files")
            .is_empty(),
        "gen2 is now empty"
    );
}

#[test]
fn purging_a_generation_removes_all_three_of_its_tables() {
    let (_dir, conn) = migrated_db();
    remote_code::replace_generation(&conn, REPO, "gen1", &projection(3)).expect("replace");

    remote_code::purge_generation(&conn, REPO, "gen1").expect("purge");

    assert!(
        remote_code::files(&conn, REPO, "gen1")
            .expect("files")
            .is_empty()
    );
    assert!(
        remote_code::symbols(&conn, REPO, "gen1")
            .expect("symbols")
            .is_empty()
    );
    assert!(
        remote_code::edges(&conn, REPO, "gen1")
            .expect("edges")
            .is_empty()
    );
}

#[test]
fn an_empty_projection_is_a_real_state_not_an_error() {
    let (_dir, conn) = migrated_db();

    remote_code::replace_generation(&conn, REPO, "gen1", &Projection::default())
        .expect("an authoritative empty generation is writable");

    assert!(
        remote_code::files(&conn, REPO, "gen1")
            .expect("files")
            .is_empty()
    );
}

#[test]
fn one_repos_projection_is_not_another_repos() {
    let (_dir, conn) = migrated_db();
    remote_code::replace_generation(&conn, REPO, "gen1", &projection(3)).expect("replace");

    assert!(
        remote_code::files(&conn, "Falconiere/other", "gen1")
            .expect("files")
            .is_empty()
    );
}

#[test]
fn the_database_refuses_a_relation_outside_the_declared_vocabulary() {
    let (_dir, conn) = migrated_db();

    conn.execute_batch(
        "INSERT INTO remote_code_edge(repo, generation_id, rel, src_path, dst_path) \
         VALUES ('r', 'g', 'calls', 'a', 'b');",
    )
    .expect_err("the CHECK constraint refuses an unknown relation");
}
