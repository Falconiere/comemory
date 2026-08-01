//! Test mirror for `src/document/writer.rs`, run against real fixture
//! files copied into a disposable temp source root (the writer must
//! never be handed the checked-in fixtures directly since some cases
//! here mutate content and permissions).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use comemory::document::DocumentFormat;
use comemory::document::writer::{self, UpdateOutcome};
use comemory::source::classify::Classification;
use comemory::source::discover::Candidate;
use comemory::store::sources::SourceRootUpsert;
use comemory::store::{connection, document_fts, documents, sources};
use rusqlite::{Connection, params};
use tempfile::TempDir;

const GUIDE_MD: &[u8] = include_bytes!("common/fixtures/docs/guide.md");
const CHANGELOG_TXT: &[u8] = include_bytes!("common/fixtures/docs/changelog.txt");
const PAGE_HTML: &[u8] = include_bytes!("common/fixtures/docs/page.html");

const SOURCE_ID: &str = "source-under-test";
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// Open a fresh `comemory.db` and seed the `source_roots` row `SOURCE_ID`
/// every `source_files` row here is foreign-keyed to.
fn open_db(tmp: &TempDir) -> Connection {
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");
    sources::upsert(
        &conn,
        SourceRootUpsert {
            id: SOURCE_ID,
            canonical_path: "/does/not/matter",
            kind: "dir",
            repo: None,
            created_at: "2026-01-01T00:00:00.000000000Z",
            updated_at: "2026-01-01T00:00:00.000000000Z",
        },
    )
    .expect("seed source_roots row");
    conn
}

fn write_fixture(tmp: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = tmp.path().join(name);
    fs::write(&path, bytes).expect("write fixture copy");
    path
}

fn candidate(relative: &str, absolute: &Path, format: DocumentFormat) -> Candidate {
    Candidate {
        relative_path: PathBuf::from(relative),
        absolute_path: absolute.to_path_buf(),
        classification: Classification::Document(format),
    }
}

fn index(
    conn: &mut Connection,
    path: &Path,
    relative: &str,
    format: DocumentFormat,
) -> UpdateOutcome {
    let c = candidate(relative, path, format);
    writer::update_file(conn, SOURCE_ID, None, &c, MAX_BYTES).expect("update_file")
}

fn bump_mtime(path: &Path, when: SystemTime) {
    let file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open for mtime bump");
    file.set_modified(when).expect("set_modified");
}

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
fn unchanged_size_and_mtime_skip_without_any_write() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "changelog.txt", CHANGELOG_TXT);

    let first = index(&mut conn, &path, "changelog.txt", DocumentFormat::Txt);
    assert!(matches!(first, UpdateOutcome::Indexed { .. }));
    let before = sources::list_files_by_source(&conn, SOURCE_ID).expect("list")[0].clone();

    let second = index(&mut conn, &path, "changelog.txt", DocumentFormat::Txt);
    assert_eq!(second, UpdateOutcome::Unchanged);
    let after = sources::list_files_by_source(&conn, SOURCE_ID).expect("list")[0].clone();
    assert_eq!(
        before, after,
        "exact-fingerprint skip must not touch the row at all"
    );
}

#[test]
fn mtime_touch_without_content_change_touches_fingerprint_but_skips_reextraction() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "changelog.txt", CHANGELOG_TXT);

    let first = index(&mut conn, &path, "changelog.txt", DocumentFormat::Txt);
    let UpdateOutcome::Indexed { document_id } = first else {
        panic!("expected Indexed")
    };
    let file_before = sources::list_files_by_source(&conn, SOURCE_ID).expect("list")[0].clone();
    let doc_before = documents::get_document(&conn, &document_id)
        .expect("get")
        .expect("exists");

    bump_mtime(&path, SystemTime::now() + Duration::from_secs(120));

    let second = index(&mut conn, &path, "changelog.txt", DocumentFormat::Txt);
    assert_eq!(
        second,
        UpdateOutcome::Unchanged,
        "unchanged content must skip re-extraction"
    );

    let file_after = sources::list_files_by_source(&conn, SOURCE_ID).expect("list")[0].clone();
    assert_ne!(
        file_after.mtime, file_before.mtime,
        "fingerprint mtime must be touched"
    );
    assert_eq!(
        file_after.sha256, file_before.sha256,
        "content hash is unchanged"
    );

    let doc_after = documents::get_document(&conn, &document_id)
        .expect("get")
        .expect("exists");
    assert_eq!(
        doc_before.updated_at, doc_after.updated_at,
        "documents row must not be rewritten when content is unchanged"
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

#[test]
fn oversized_file_is_recorded_too_large_without_extraction() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "changelog.txt", CHANGELOG_TXT);

    let c = candidate("changelog.txt", &path, DocumentFormat::Txt);
    let outcome = writer::update_file(&mut conn, SOURCE_ID, None, &c, 4).expect("update_file");
    assert_eq!(outcome, UpdateOutcome::TooLarge);

    let files = sources::list_files_by_source(&conn, SOURCE_ID).expect("list");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status, "too_large");
    assert!(
        files[0].sha256.is_none(),
        "oversized file must never be hashed"
    );

    let doc_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
        .expect("count documents");
    assert_eq!(doc_count, 0, "no documents row for a too-large file");
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
