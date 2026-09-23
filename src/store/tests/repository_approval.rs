#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `repository_approval` against a real migrated database: what an offline run
//! is allowed to conclude about a repository, and what revocation must do.

use comemory::store::repository_approval;
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

const AT: &str = "2026-09-23T10:00:00Z";
const LATER: &str = "2026-09-23T11:00:00Z";

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

fn pairs(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(l, c)| ((*l).to_string(), (*c).to_string()))
        .collect()
}

#[test]
fn nothing_is_approved_before_a_policy_has_ever_loaded() {
    let (_dir, conn) = migrated_db();

    assert_eq!(
        repository_approval::canonical_for(&conn, "Falconiere/comemory").expect("read"),
        None,
        "a machine that never joined a workspace shares nothing"
    );
}

#[test]
fn a_label_resolves_to_its_canonical_name_whatever_its_case() {
    let (_dir, conn) = migrated_db();
    repository_approval::replace_all(
        &conn,
        &pairs(&[("Falconiere/comemory", "Falconiere/comemory")]),
        AT,
    )
    .expect("write");

    for typed in [
        "Falconiere/comemory",
        "falconiere/comemory",
        "  FALCONIERE/COMEMORY  ",
    ] {
        assert_eq!(
            repository_approval::canonical_for(&conn, typed)
                .expect("read")
                .as_deref(),
            Some("Falconiere/comemory"),
            "`{typed}` names the same repository"
        );
    }
}

#[test]
fn a_local_label_mapped_to_another_name_resolves_to_that_name() {
    let (_dir, conn) = migrated_db();
    // The case the whole table exists for: the operator typed `my-notes`, the
    // policy says that is `Falconiere/comemory`. Two machines with different
    // local labels must still compute one shared id.
    repository_approval::replace_all(&conn, &pairs(&[("my-notes", "Falconiere/comemory")]), AT)
        .expect("write");

    assert_eq!(
        repository_approval::canonical_for(&conn, "my-notes")
            .expect("read")
            .as_deref(),
        Some("Falconiere/comemory")
    );
}

#[test]
fn a_revoked_repository_stops_resolving() {
    let (_dir, conn) = migrated_db();
    repository_approval::replace_all(
        &conn,
        &pairs(&[
            ("Falconiere/comemory", "Falconiere/comemory"),
            ("Falconiere/other", "Falconiere/other"),
        ]),
        AT,
    )
    .expect("first load");
    assert!(
        repository_approval::canonical_for(&conn, "Falconiere/other")
            .expect("read")
            .is_some(),
        "it was approved a moment ago, so the revocation below is not vacuous"
    );

    // The second load no longer lists it: the map is replaced, not merged, so
    // a stale row cannot keep saying a revoked repository is shareable.
    repository_approval::replace_all(
        &conn,
        &pairs(&[("Falconiere/comemory", "Falconiere/comemory")]),
        LATER,
    )
    .expect("second load");

    assert_eq!(
        repository_approval::canonical_for(&conn, "Falconiere/other").expect("read"),
        None,
        "revoked"
    );
    assert_eq!(
        repository_approval::canonical_for(&conn, "Falconiere/comemory")
            .expect("read")
            .as_deref(),
        Some("Falconiere/comemory"),
        "and the one still approved is untouched"
    );
}

#[test]
fn an_empty_label_is_neither_stored_nor_looked_up() {
    let (_dir, conn) = migrated_db();
    // A blank `source_roots.repo` must not match a row, and must not BECOME
    // one: stored under an empty key it would be a row nothing can ever read.
    repository_approval::replace_all(&conn, &pairs(&[("   ", "Falconiere/comemory")]), AT)
        .expect("write");

    assert_eq!(repository_approval::len(&conn).expect("count"), 0);
    assert_eq!(
        repository_approval::canonical_for(&conn, "").expect("read"),
        None
    );
    assert_eq!(
        repository_approval::canonical_for(&conn, "   ").expect("read"),
        None
    );
}

#[test]
fn the_size_of_the_map_tells_unloaded_from_unapproved() {
    let (_dir, conn) = migrated_db();
    assert_eq!(
        repository_approval::len(&conn).expect("count"),
        0,
        "no policy has loaded"
    );

    repository_approval::replace_all(
        &conn,
        &pairs(&[("Falconiere/comemory", "Falconiere/comemory")]),
        AT,
    )
    .expect("write");

    assert_eq!(repository_approval::len(&conn).expect("count"), 1);
    assert_eq!(
        repository_approval::canonical_for(&conn, "Falconiere/other").expect("read"),
        None,
        "a policy HAS loaded; this repository is simply not approved"
    );
}
