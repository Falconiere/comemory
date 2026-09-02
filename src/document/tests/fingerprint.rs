#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    clippy::print_stderr
)]
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

use crate::test_common::document_writer_support as support;
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

    bump_mtime(&path, SystemTime::now() + Duration::from_mins(2));

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
    let root = fs::canonicalize(tmp.path()).expect("canonicalize root");

    let c = candidate("changelog.txt", &path, DocumentFormat::Txt);
    let outcome =
        writer::update_file(&mut conn, SOURCE_ID, None, &c, &root, 4).expect("update_file");
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

/// Regression: without the `existing.status` guard, an unchanged
/// (still-oversized) fingerprint on the second run would short-circuit
/// straight to `Unchanged` — silently hiding a persistent `--strict`
/// failure after its first report.
#[test]
fn oversized_file_is_still_reported_too_large_on_an_unchanged_second_run() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "changelog.txt", CHANGELOG_TXT);
    let root = fs::canonicalize(tmp.path()).expect("canonicalize root");
    let c = candidate("changelog.txt", &path, DocumentFormat::Txt);

    let first = writer::update_file(&mut conn, SOURCE_ID, None, &c, &root, 4).expect("first run");
    assert_eq!(first, UpdateOutcome::TooLarge);

    // Same file, untouched: size and mtime are byte-identical to what
    // was just recorded.
    let second = writer::update_file(&mut conn, SOURCE_ID, None, &c, &root, 4).expect("second run");
    assert_eq!(
        second,
        UpdateOutcome::TooLarge,
        "an unchanged fingerprint on a too_large row must re-report TooLarge, not Unchanged"
    );
    assert_eq!(
        sources::list_files_by_source(&conn, SOURCE_ID).expect("list")[0].status,
        "too_large"
    );
}

/// Regression: same bug as the too_large case above, for a file that
/// fails to *read* (permission denied) rather than one that's oversized
/// — the persistent read failure must keep being reported on every run.
#[cfg(unix)]
#[test]
fn unreadable_file_is_still_reported_as_error_on_an_unchanged_second_run() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "changelog.txt", CHANGELOG_TXT);
    let root = fs::canonicalize(tmp.path()).expect("canonicalize root");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod 000");
    if fs::read(&path).is_ok() {
        // uid 0 bypasses mode bits, so the "unreadable" precondition cannot
        // be staged under root. Say so on stderr (visible with --nocapture
        // and in every failure report) instead of passing silently.
        eprintln!(
            "SKIPPED: chmod 000 left {} readable (running as root?)",
            path.display()
        );
        return;
    }
    let c = candidate("changelog.txt", &path, DocumentFormat::Txt);

    let first =
        writer::update_file(&mut conn, SOURCE_ID, None, &c, &root, MAX_BYTES).expect("first run");
    assert!(
        matches!(first, UpdateOutcome::Error(_)),
        "unreadable file must report Error, got {first:?}"
    );

    // Same file, still chmod 000: size and mtime are unchanged from
    // what was just recorded with status "error".
    let second =
        writer::update_file(&mut conn, SOURCE_ID, None, &c, &root, MAX_BYTES).expect("second run");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("restore perms");
    assert!(
        matches!(second, UpdateOutcome::Error(_)),
        "an unchanged fingerprint on an error row must re-attempt and report Error again, \
         not Unchanged; got {second:?}"
    );
    assert_eq!(
        sources::list_files_by_source(&conn, SOURCE_ID).expect("list")[0].status,
        "error"
    );
}
