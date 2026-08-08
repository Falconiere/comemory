#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/api/rebuild/documents.rs`. Exercised directly
//! through an `ATTACH`ed real "old" database via the public
//! `copy_preserved_tables_from_old` entry point — not through the CLI
//! binary; crate-root `tests/cli__rebuild_3.rs` covers the full `comemory
//! rebuild` workflow end to end with a real indexed document corpus.

use comemory::document::DocumentFormat;
use comemory::document::writer::UpdateOutcome;
use comemory::store::connection;
use comemory::store::sources::{self, SourceRootUpsert};
use rusqlite::Connection;
use tempfile::TempDir;

use crate::api::rebuild::copy::copy_preserved_tables_from_old;
use crate::test_common::document_writer_support as support;

/// Open a fresh, fully-migrated `comemory.db` at `tmp` and seed its
/// `source_roots` row for [`support::SOURCE_ID`] — the FK
/// `copy_document_tables_inner`'s `source_files` copy requires, mirroring
/// what `source::mirror::reconcile` populates in the real
/// `rebuild::build_new_db` flow before this copy runs.
fn open_new_db_with_source_root(tmp: &TempDir) -> Connection {
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open new db");
    sources::upsert(
        &conn,
        SourceRootUpsert {
            id: support::SOURCE_ID,
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

#[test]
fn copy_document_tables_inner_copies_source_files_documents_chunks_and_fts() {
    let old_dir = TempDir::new().expect("old tempdir");
    let mut old_conn = support::open_db(&old_dir);
    let path = support::write_fixture(&old_dir, "changelog.txt", support::CHANGELOG_TXT);
    let outcome = support::index(&mut old_conn, &path, "changelog.txt", DocumentFormat::Txt);
    assert!(
        matches!(outcome, UpdateOutcome::Indexed { .. }),
        "{outcome:?}"
    );
    drop(old_conn);

    let new_dir = TempDir::new().expect("new tempdir");
    let mut new_conn = open_new_db_with_source_root(&new_dir);

    copy_preserved_tables_from_old(&mut new_conn, &old_dir.path().join("comemory.db"))
        .expect("copy_preserved_tables_from_old");

    let source_files: i64 = new_conn
        .query_row("SELECT count(*) FROM source_files", [], |r| r.get(0))
        .expect("count source_files");
    assert_eq!(source_files, 1, "source_files row must be copied");

    let (title, revision_hash): (String, String) = new_conn
        .query_row("SELECT title, revision_hash FROM documents", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("documents row must be copied");
    assert_eq!(title, "changelog", "TXT has no heading; title is the stem");
    assert_eq!(revision_hash.len(), 64, "full sha256 hex digest");

    let chunks: i64 = new_conn
        .query_row("SELECT count(*) FROM document_chunks", [], |r| r.get(0))
        .expect("count document_chunks");
    assert!(chunks > 0, "expected at least one chunk copied");

    let fts_hits: i64 = new_conn
        .query_row(
            "SELECT count(*) FROM document_fts WHERE document_fts MATCH 'changelog'",
            [],
            |r| r.get(0),
        )
        .expect("query document_fts");
    assert_eq!(fts_hits, 1, "document_fts row must be copied and matchable");
}

/// A pre-v13 source lacks all four document-domain tables outright — the
/// copy must skip them without erroring, the same graceful-absence
/// contract `copy.rs`'s older tables already have.
#[test]
fn copy_document_tables_inner_is_a_no_op_when_the_old_db_predates_v13() {
    let old_dir = TempDir::new().expect("old tempdir");
    let old_conn = connection::open(old_dir.path().join("comemory.db")).expect("open old db");
    old_conn
        .execute_batch(
            "DROP TABLE document_fts; DROP TABLE document_chunks; \
             DROP TABLE documents; DROP TABLE source_files;",
        )
        .expect("drop the v13 document tables to simulate a pre-v13 source");
    drop(old_conn);

    let new_dir = TempDir::new().expect("new tempdir");
    let mut new_conn = connection::open(new_dir.path().join("comemory.db")).expect("open new db");

    copy_preserved_tables_from_old(&mut new_conn, &old_dir.path().join("comemory.db"))
        .expect("copy must succeed even when the old DB lacks the v13 tables");

    let documents: i64 = new_conn
        .query_row("SELECT count(*) FROM documents", [], |r| r.get(0))
        .expect("count documents");
    assert_eq!(documents, 0);
}
