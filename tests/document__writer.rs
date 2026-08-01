//! Test mirror for `src/document/writer.rs`: the full extract →
//! one-transaction write path plus tombstone reconciliation. The
//! fingerprint-skip ladder (`src/document/fingerprint.rs`) has its own
//! mirror at `tests/document__fingerprint.rs`; both share fixture
//! plumbing via `common/document_writer_support.rs`.

use std::collections::HashSet;
use std::fs;

use comemory::document::DocumentFormat;
use comemory::document::writer::{self, UpdateOutcome};
use comemory::store::sources;
use comemory::store::{document_fts, documents};
use rusqlite::{Connection, params};
use tempfile::TempDir;

#[path = "common/document_writer_support.rs"]
mod support;
use support::*;

/// Real fixtures only this mirror indexes — `changelog.txt` (shared
/// with `tests/document__fingerprint.rs`) lives in `support`.
const GUIDE_MD: &[u8] = include_bytes!("common/fixtures/docs/guide.md");
const PAGE_HTML: &[u8] = include_bytes!("common/fixtures/docs/page.html");

fn chunk_count(conn: &Connection, document_id: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM document_chunks WHERE document_id = ?1",
        params![document_id],
        |r| r.get(0),
    )
    .expect("count chunks")
}

fn fts_count(conn: &Connection, document_id: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM document_fts WHERE document_id = ?1",
        params![document_id],
        |r| r.get(0),
    )
    .expect("count fts rows")
}

#[test]
fn first_index_creates_all_rows() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);

    let outcome = index(&mut conn, &path, "guide.md", DocumentFormat::Markdown);
    let UpdateOutcome::Indexed { document_id } = outcome else {
        panic!("expected Indexed, got {outcome:?}");
    };

    let files = sources::list_files_by_source(&conn, SOURCE_ID).expect("list files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status, "indexed");
    assert_eq!(files[0].classification, "document");
    assert!(files[0].sha256.is_some());

    let doc = documents::get_document(&conn, &document_id)
        .expect("get document")
        .expect("document row exists");
    assert_eq!(doc.title, "Comemory CLI Guide");

    let chunks = chunk_count(&conn, &document_id);
    assert!(chunks > 1, "guide.md has multiple heading sections");
    assert_eq!(
        fts_count(&conn, &document_id),
        chunks,
        "one fts row per chunk"
    );
}

#[test]
fn content_edit_at_same_path_preserves_document_id_and_replaces_chunks() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "notes.md", b"# First\n\noriginal content here.\n");

    let first = index(&mut conn, &path, "notes.md", DocumentFormat::Markdown);
    let UpdateOutcome::Indexed { document_id: id_1 } = first else {
        panic!("expected Indexed")
    };

    fs::write(
        &path,
        b"# Second\n\nrewritten content, totally different.\n",
    )
    .expect("rewrite");
    let second = index(&mut conn, &path, "notes.md", DocumentFormat::Markdown);
    let UpdateOutcome::Indexed { document_id: id_2 } = second else {
        panic!("expected Indexed")
    };
    assert_eq!(
        id_1, id_2,
        "document id must survive a content edit at the same path"
    );

    let doc = documents::get_document(&conn, &id_2)
        .expect("get")
        .expect("exists");
    assert_eq!(doc.title, "Second");

    let mut stmt = conn
        .prepare("SELECT text FROM document_chunks WHERE document_id = ?1 ORDER BY ordinal")
        .expect("prepare");
    let texts: Vec<String> = stmt
        .query_map(params![id_2], |r| r.get(0))
        .expect("query")
        .collect::<std::result::Result<_, _>>()
        .expect("rows");
    assert!(texts.iter().any(|t| t.contains("rewritten content")));
    assert!(!texts.iter().any(|t| t.contains("original content")));
}

#[cfg(unix)]
#[test]
fn unreadable_file_after_a_good_index_records_error_and_keeps_prior_rows() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);

    let first = index(&mut conn, &path, "guide.md", DocumentFormat::Markdown);
    let UpdateOutcome::Indexed { document_id } = first else {
        panic!("expected Indexed")
    };
    let doc_before = documents::get_document(&conn, &document_id)
        .expect("get")
        .expect("exists");
    let chunks_before = chunk_count(&conn, &document_id);

    // Resize (forces past the fingerprint shortcut) then revoke all
    // access so the writer's own `fs::read` fails.
    let mut resized = GUIDE_MD.to_vec();
    resized.extend_from_slice(b"\nmore content to change the size\n");
    fs::write(&path, &resized).expect("resize");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod 000");

    let second = index(&mut conn, &path, "guide.md", DocumentFormat::Markdown);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("restore perms");

    assert!(
        matches!(second, UpdateOutcome::Error(_)),
        "unreadable file must report Error, got {second:?}"
    );

    let files = sources::list_files_by_source(&conn, SOURCE_ID).expect("list");
    assert_eq!(files[0].status, "error");
    assert!(files[0].error.is_some());

    let doc_after = documents::get_document(&conn, &document_id)
        .expect("get")
        .expect("prior revision must still exist");
    assert_eq!(
        doc_before, doc_after,
        "prior documents row must be untouched"
    );
    assert_eq!(
        chunks_before,
        chunk_count(&conn, &document_id),
        "prior chunks must be untouched"
    );
}

/// Index guide.md + changelog.txt under [`SOURCE_ID`] and return the
/// live connection plus each document's id — shared setup for the
/// `reconcile_deletions` tests below.
fn seed_two_sources(tmp: &TempDir) -> (Connection, String, String) {
    let mut conn = open_db(tmp);
    let path_a = write_fixture(tmp, "guide.md", GUIDE_MD);
    let path_b = write_fixture(tmp, "changelog.txt", CHANGELOG_TXT);
    let UpdateOutcome::Indexed { document_id: doc_a } =
        index(&mut conn, &path_a, "guide.md", DocumentFormat::Markdown)
    else {
        panic!("expected Indexed")
    };
    let UpdateOutcome::Indexed { document_id: doc_b } =
        index(&mut conn, &path_b, "changelog.txt", DocumentFormat::Txt)
    else {
        panic!("expected Indexed")
    };
    (conn, doc_a, doc_b)
}

#[test]
fn reconcile_deletions_removes_derived_rows_and_tombstones() {
    let tmp = TempDir::new().expect("tempdir");
    let (mut conn, doc_a, doc_b) = seed_two_sources(&tmp);

    // A fresh, authoritative scan only saw guide.md this time.
    let mut seen = HashSet::new();
    seen.insert("guide.md".to_string());
    let removed = writer::reconcile_deletions(&mut conn, SOURCE_ID, &seen).expect("reconcile");
    assert_eq!(removed, 1);

    let files = sources::list_files_by_source(&conn, SOURCE_ID).expect("list");
    let changelog_row = files
        .iter()
        .find(|r| r.relative_path == "changelog.txt")
        .expect("row kept as tombstone");
    assert_eq!(changelog_row.status, "deleted");
    let guide_row = files
        .iter()
        .find(|r| r.relative_path == "guide.md")
        .expect("guide row present");
    assert_eq!(
        guide_row.status, "indexed",
        "reachable file must be untouched"
    );

    assert!(
        documents::get_document(&conn, &doc_b)
            .expect("get")
            .is_none(),
        "tombstoned document row removed"
    );
    assert!(
        documents::get_document(&conn, &doc_a)
            .expect("get")
            .is_some(),
        "kept document row survives"
    );
    assert_eq!(
        fts_count(&conn, &doc_b),
        0,
        "tombstoned document's fts rows removed"
    );
}

#[test]
fn reconcile_deletions_is_idempotent_for_already_deleted_rows() {
    let tmp = TempDir::new().expect("tempdir");
    let (mut conn, _doc_a, _doc_b) = seed_two_sources(&tmp);

    let mut seen = HashSet::new();
    seen.insert("guide.md".to_string());
    writer::reconcile_deletions(&mut conn, SOURCE_ID, &seen).expect("first reconcile");

    let removed_again =
        writer::reconcile_deletions(&mut conn, SOURCE_ID, &seen).expect("second reconcile");
    assert_eq!(removed_again, 0, "already-deleted rows are left alone");
}

#[test]
fn fts_match_finds_a_guide_phrase_and_ranks_it_first() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path_a = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let path_b = write_fixture(&tmp, "changelog.txt", CHANGELOG_TXT);
    let path_c = write_fixture(&tmp, "page.html", PAGE_HTML);

    let UpdateOutcome::Indexed {
        document_id: guide_id,
    } = index(&mut conn, &path_a, "guide.md", DocumentFormat::Markdown)
    else {
        panic!("expected Indexed")
    };
    index(&mut conn, &path_b, "changelog.txt", DocumentFormat::Txt);
    index(&mut conn, &path_c, "page.html", DocumentFormat::Html);

    let hits = document_fts::search(&conn, "lazy reindex debounce window", 5).expect("search");
    assert!(
        !hits.is_empty(),
        "must find the guide's troubleshooting passage"
    );
    assert_eq!(
        hits[0].document_id, guide_id,
        "on-topic chunk must rank first"
    );
}
