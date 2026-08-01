//! Test mirror for `src/document/fingerprint.rs`. Its size+mtime skip,
//! size-ceiling, and content-hash comparison are all private to the
//! module, so — same as the current `writer::update_file` mirror in
//! `tests/document__writer.rs` — these tests drive that one public
//! entry point and assert on the fingerprint-specific outcomes it
//! produces. Fixture plumbing is shared via
//! `common/document_writer_support.rs`.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use comemory::document::DocumentFormat;
use comemory::document::writer::{self, UpdateOutcome};
use comemory::store::documents;
use comemory::store::sources;
use tempfile::TempDir;

#[path = "common/document_writer_support.rs"]
mod support;
use support::*;

/// Bump `path`'s mtime without touching its content — only this
/// mirror needs to drive the fingerprint's mtime-vs-hash comparison.
fn bump_mtime(path: &Path, when: SystemTime) {
    let file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open for mtime bump");
    file.set_modified(when).expect("set_modified");
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
