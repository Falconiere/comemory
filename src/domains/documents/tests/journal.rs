#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The erased-digest barrier on local capture (#256, B-5), through the real
//! document writer over a real migrated database and a real file: once a
//! revision's payload is erased, an unchanged re-index records the share
//! blocked `erased` and journals nothing, while an edit — new bytes — shares
//! a new revision.

use std::fs;
use std::path::Path;

use comemory::domains::documents::document::DocumentFormat;
use comemory::domains::documents::document::writer::{self, UpdateOutcome};
use comemory::store::erase_rows;
use comemory::store::replica_read::Redaction;
use comemory::store::replica_redaction::{self, Reach};
use rusqlite::{Connection, params};
use tempfile::TempDir;

use crate::test_common::document_writer_support as support;

const REPO: &str = "Falconiere/comemory";
const GUIDE_MD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/common/fixtures/docs/guide.md"
));
const NOW: &str = "2026-09-25T10:00:00Z";

/// Approve `REPO` and give it `root` as its indexed root — the two rows a
/// policy load and an `index-code` leave.
fn approve(conn: &Connection, root: &Path) {
    comemory::store::repository_approval::replace_all(
        conn,
        &[(REPO.to_string(), REPO.to_string())],
        NOW,
    )
    .expect("approve");
    conn.execute(
        "INSERT INTO repo_marker (repo, root_path) VALUES (?1, ?2)",
        params![REPO, root.to_str().expect("utf8 root")],
    )
    .expect("root");
}

fn index(conn: &mut Connection, path: &Path) -> UpdateOutcome {
    let candidate = support::candidate("guide.md", path, DocumentFormat::Markdown);
    let root = fs::canonicalize(path.parent().expect("parent")).expect("canonical");
    writer::update_file(
        conn,
        support::SOURCE_ID,
        Some(REPO),
        &candidate,
        &root,
        support::MAX_BYTES,
    )
    .expect("update_file")
}

/// `(op, digest)` of every document position, oldest first.
fn document_feed(conn: &Connection) -> Vec<(String, Option<String>)> {
    let mut statement = conn
        .prepare(
            "SELECT op, payload_digest FROM replica_feed \
              WHERE entity_kind = 'document_revision' ORDER BY sequence",
        )
        .expect("prepare");
    statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

fn blocked_reason(conn: &Connection, shared_id: &str) -> Option<String> {
    conn.query_row(
        "SELECT blocked_reason FROM document_share WHERE shared_id = ?1",
        [shared_id],
        |r| r.get(0),
    )
    .expect("share row")
}

/// Erase the one shared document the way `maintenance::erase` does at the
/// store layer: its rows and `source_files` row go, its payload is blanked.
fn erase(conn: &mut Connection, shared_id: &str) -> String {
    let tx = conn.transaction().expect("begin");
    for doc in erase_rows::local_documents(&tx, shared_id).expect("lookup") {
        erase_rows::erase_local_document(&tx, &doc).expect("rows");
    }
    let erased = replica_redaction::redact(&tx, Reach::Entity("document_revision", shared_id), NOW)
        .expect("redact");
    tx.commit().expect("commit");
    assert_eq!(erased.len(), 1, "the one revision's payload");
    erased[0].clone()
}

fn setup() -> (TempDir, Connection, std::path::PathBuf, String) {
    let tmp = TempDir::new().expect("tempdir");
    let path = support::write_fixture(&tmp, "guide.md", GUIDE_MD);
    let conn = support::open_db(&tmp);
    approve(&conn, &fs::canonicalize(tmp.path()).expect("canonical"));
    let shared_id = comemory::domains::documents::share::shared_id(REPO, "guide.md");
    (tmp, conn, path, shared_id)
}

#[test]
fn an_unchanged_reindex_of_an_erased_revision_is_blocked_erased_and_journals_nothing() {
    let (_tmp, mut conn, path, shared_id) = setup();
    index(&mut conn, &path);
    let digest = erase(&mut conn, &shared_id);
    let feed_after_erase = document_feed(&conn);

    let outcome = index(&mut conn, &path);

    assert!(
        matches!(outcome, UpdateOutcome::Indexed { .. }),
        "indexed locally again: {outcome:?}"
    );
    assert_eq!(
        document_feed(&conn),
        feed_after_erase,
        "nothing journalled for the erased bytes"
    );
    assert_eq!(
        blocked_reason(&conn, &shared_id).as_deref(),
        Some(crate::domains::documents::journal::ERASED)
    );
    assert_eq!(
        replica_redaction::redaction_of(&conn, &digest).expect("redaction"),
        Some(Redaction::Erased),
        "the barrier is untouched: no bytes stored for the erased digest again"
    );
    assert_eq!(
        comemory::store::replica_outbox::count(&conn, "pending").expect("count"),
        1,
        "only the original revision's upload was ever queued"
    );
}

#[test]
fn an_edit_after_an_erase_shares_a_new_revision() {
    let (_tmp, mut conn, path, shared_id) = setup();
    index(&mut conn, &path);
    let erased = erase(&mut conn, &shared_id);
    index(&mut conn, &path);

    let mut edited = GUIDE_MD.to_vec();
    edited.extend_from_slice(b"\n\n## After the erase\n\nA paragraph written later.\n");
    fs::write(&path, edited).expect("edit the file");
    let outcome = index(&mut conn, &path);

    assert!(
        matches!(outcome, UpdateOutcome::Indexed { .. }),
        "{outcome:?}"
    );
    let feed = document_feed(&conn);
    let (op, digest) = feed.last().expect("a position").clone();
    assert_eq!(op, "upsert");
    let digest = digest.expect("an upsert names a payload");
    assert_ne!(digest, erased, "new bytes, new digest");
    assert_eq!(
        replica_redaction::redaction_of(&conn, &digest).expect("redaction"),
        None,
        "the new revision keeps its bytes"
    );
    assert_eq!(blocked_reason(&conn, &shared_id), None, "shared again");
}
