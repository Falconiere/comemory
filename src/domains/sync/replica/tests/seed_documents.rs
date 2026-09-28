#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Seeding documents indexed before their repository was approved (or
//! before the journal existed): a real index, no `document_share` row until
//! approval, then a real seed once it exists.

use std::fs;

use tempfile::TempDir;

use crate::domains::documents::document::DocumentFormat;
use crate::domains::documents::document::writer;
use crate::domains::documents::replica_payload::DOCUMENT_ENTITY_KIND;
use crate::domains::documents::share;
use crate::domains::documents::source::classify::Classification;
use crate::domains::documents::source::discover::Candidate;
use crate::domains::sync::replica::seed_documents;
use crate::store::sources::SourceRootUpsert;
use crate::store::{
    code_row, document_share, documents, replica_read, repository_approval, sources,
};

use crate::domains::sync::replica::test_support as support;
use support::Home;

const SOURCE_ID: &str = "seed-documents-under-test";
const LABEL: &str = "legacy-corpus";
const CANONICAL: &str = "Falconiere/legacy-corpus";

/// Index one real markdown file with no repository approved yet: the writer
/// journals nothing (`record_revision` withholds), so the document exists
/// with no `document_share` row — exactly the state a legacy database, or a
/// document indexed before its label was approved, is in.
fn index_unapproved(home: &mut Home, root: &TempDir) -> String {
    let path = root.path().join("notes.md");
    fs::write(&path, "# Notes\n\nSeeded before the label was approved.\n").expect("write fixture");
    sources::upsert(
        &home.conn,
        SourceRootUpsert {
            id: SOURCE_ID,
            canonical_path: &root.path().to_string_lossy(),
            kind: "dir",
            repo: Some(LABEL),
            created_at: "2026-09-25T10:00:00Z",
            updated_at: "2026-09-25T10:00:00Z",
        },
    )
    .expect("seed source_roots row");
    let candidate = Candidate {
        relative_path: "notes.md".into(),
        absolute_path: path,
        classification: Classification::Document(DocumentFormat::Markdown),
    };
    writer::update_file(
        &mut home.conn,
        SOURCE_ID,
        Some(LABEL),
        &candidate,
        root.path(),
        1 << 20,
    )
    .expect("index");
    let ids = documents::document_ids_for_source(&home.conn, SOURCE_ID).expect("ids");
    assert_eq!(ids.len(), 1, "one document indexed");
    ids[0].clone()
}

fn approve(home: &Home, root: &TempDir) {
    repository_approval::replace_all(
        &home.conn,
        &[(LABEL.to_string(), CANONICAL.to_string())],
        "2026-09-25T10:05:00Z",
    )
    .expect("approve");
    // `shared_name` resolves the indexed root by the SOURCE's raw label, not
    // the canonical name — the same requirement a document capture at index
    // time has.
    code_row::upsert_repo_root(&home.conn, LABEL, &root.path().to_string_lossy())
        .expect("register repo root");
}

#[test]
fn a_document_indexed_before_approval_is_seeded_once_approved() {
    let mut home = Home::new();
    let root = TempDir::new().expect("tempdir");
    let document_id = index_unapproved(&mut home, &root);
    assert_eq!(
        document_share::by_document(&home.conn, &document_id).expect("lookup"),
        None,
        "withheld: no approval existed at index time"
    );

    let mut ctx = home.ctx();
    let unapproved_scan = seed_documents::advance(&mut ctx).expect("advance before approval");
    assert!(
        unapproved_scan.complete(),
        "nothing left withheld to revisit yet"
    );
    assert_eq!(
        document_share::by_document(&home.conn, &document_id).expect("lookup"),
        None
    );

    approve(&home, &root);
    let mut ctx = home.ctx();
    let after_approval = seed_documents::advance(&mut ctx).expect("advance after approval");
    assert!(after_approval.complete());

    let share = document_share::by_document(&home.conn, &document_id)
        .expect("lookup")
        .expect("now shared");
    assert_eq!(share.repo, CANONICAL);
    let shared_id = share::shared_id(CANONICAL, "notes.md");
    assert_eq!(share.shared_id, shared_id);
    let revision = replica_read::revision(&home.conn, DOCUMENT_ENTITY_KIND, &shared_id)
        .expect("revision lookup")
        .expect("journalled");
    assert!(revision.payload_digest.is_some());
}

#[test]
fn seeding_twice_after_approval_journals_no_second_revision() {
    let mut home = Home::new();
    let root = TempDir::new().expect("tempdir");
    let document_id = index_unapproved(&mut home, &root);
    approve(&home, &root);

    let mut ctx = home.ctx();
    seed_documents::advance(&mut ctx).expect("first pass");
    let head_after_first = replica_read::head(&home.conn).expect("head");

    let mut ctx = home.ctx();
    seed_documents::advance(&mut ctx).expect("second pass");
    assert_eq!(
        replica_read::head(&home.conn).expect("head"),
        head_after_first,
        "already-shared document is skipped, not re-journalled"
    );
    let _ = document_id;
}

#[test]
fn a_file_changed_since_indexing_is_left_to_the_next_index_run() {
    let mut home = Home::new();
    let root = TempDir::new().expect("tempdir");
    let document_id = index_unapproved(&mut home, &root);
    approve(&home, &root);

    // The file on disk no longer matches `documents.revision_hash`.
    fs::write(
        root.path().join("notes.md"),
        "# Notes\n\nChanged after indexing.\n",
    )
    .expect("mutate fixture");

    let mut ctx = home.ctx();
    let progress = seed_documents::advance(&mut ctx).expect("advance");
    assert!(progress.complete(), "the scan still finishes");
    assert_eq!(
        document_share::by_document(&home.conn, &document_id).expect("lookup"),
        None,
        "a changed file is left to comemory index, not journalled with stale bytes"
    );
}

#[test]
fn a_fresh_data_dir_completes_with_nothing_to_seed() {
    let mut home = Home::new();
    let mut ctx = home.ctx();
    let progress = seed_documents::advance(&mut ctx).expect("advance");
    assert!(progress.complete());
}

#[test]
fn a_file_larger_than_the_sniff_window_is_seeded_without_panicking() {
    let mut home = Home::new();
    let root = TempDir::new().expect("tempdir");
    // classify()'s contract requires its content_head bounded to SNIFF_WINDOW
    // (8192 bytes); a document this large regressed that (#256).
    let large = "# Large\n\n".to_string() + &"word ".repeat(4000);
    let path = root.path().join("notes.md");
    fs::write(&path, &large).expect("write large fixture");
    sources::upsert(
        &home.conn,
        SourceRootUpsert {
            id: SOURCE_ID,
            canonical_path: &root.path().to_string_lossy(),
            kind: "dir",
            repo: Some(LABEL),
            created_at: "2026-09-25T10:00:00Z",
            updated_at: "2026-09-25T10:00:00Z",
        },
    )
    .expect("seed source_roots row");
    let candidate = crate::domains::documents::source::discover::Candidate {
        relative_path: "notes.md".into(),
        absolute_path: path,
        classification: crate::domains::documents::source::classify::Classification::Document(
            crate::domains::documents::document::DocumentFormat::Markdown,
        ),
    };
    writer::update_file(
        &mut home.conn,
        SOURCE_ID,
        Some(LABEL),
        &candidate,
        root.path(),
        1 << 20,
    )
    .expect("index");
    approve(&home, &root);

    let mut ctx = home.ctx();
    let progress = seed_documents::advance(&mut ctx).expect("advance did not panic");
    assert!(progress.complete());
}
