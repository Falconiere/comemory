#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The reader half of document replication over the real surface: one engine
//! indexes this repository's own guides, another imports what it journalled
//! over real HTTP, and both are asked what they can find.
//!
//! Covers #253 AC-8 (an import writes no local row), AC-9 (a machine with no
//! registration at all still answers, with provenance), AC-12 (the local side
//! wins, and a revoked approval stops answering) and AC-14 (links cross without
//! becoming graph edges).
//!
//! The writer half — the portable identity and what a rename journals — is
//! `replica_documents.rs`.

#[path = "common/replica_support.rs"]
mod replica_support;

use replica_support::{
    DOC_REPO, Engine, approve_docs, docs_tree, document_envelope, index_docs_cli,
    journalled_revision, shared_names,
};

/// The guide every case keys on, as a pulled revision names it: relative to
/// the REPOSITORY.
const GUIDE: &str = "docs/guides/cloud-sync.md";

/// The same guide as a LOCAL hit names it: relative to the registered source
/// root, which is `docs/guides` here. The two shapes are not the same value,
/// which is why the share mapping — not the path — is what relates the sides.
const LOCAL_GUIDE: &str = "cloud-sync.md";

/// A term the real cloud-sync guide uses.
const TERM: &str = "workspace";

/// An engine that has indexed this repository's guides, and the payload it
/// journalled for [`GUIDE`].
fn author(workspace: &std::path::Path) -> (Engine, std::path::PathBuf, serde_json::Value) {
    let root = docs_tree(workspace, "author-checkout");
    let engine = Engine::spawn(&[]);
    approve_docs(&engine, &root);
    index_docs_cli(&engine, &root.join("docs/guides"));
    let shared_id = shared_names(&engine.data_dir())
        .into_iter()
        .find(|(_, path)| path == GUIDE)
        .expect("the guide was shared")
        .0;
    let payload = journalled_revision(&engine.data_dir(), &shared_id);
    (engine, root, payload)
}

/// Every document hit `comemory search` returns on `engine`.
fn document_hits(engine: &Engine, query: &str) -> Vec<serde_json::Value> {
    let out = engine.cli(&["search", query, "--only", "document"]);
    out["items"]
        .as_array()
        .unwrap_or_else(|| panic!("document hits: {out}"))
        .clone()
}

/// One table's row count in `engine`'s database.
fn rows(engine: &Engine, table: &str) -> i64 {
    engine
        .db()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .expect("count")
}

#[test]
fn a_machine_with_no_registration_answers_from_what_it_pulled() {
    let workspace = tempfile::tempdir().expect("workspace");
    let (_author, _root, payload) = author(workspace.path());
    let reader = Engine::spawn(&[]);
    approve_docs(&reader, workspace.path());

    let (status, body) = reader.post(
        "/api/v1/sync/replica/import",
        &document_envelope("op-20260923-read0001", &payload),
    );

    assert_eq!(status, 200, "import: {body}");
    assert_eq!(body["data"]["results"][0]["disposition"], "accepted");
    assert_eq!(
        rows(&reader, "source_roots"),
        0,
        "no registration was invented to hold it"
    );
    assert_eq!(rows(&reader, "documents"), 0, "nor a local document row");
    let hits = document_hits(&reader, TERM);
    let hit = hits
        .iter()
        .find(|h| h["citation"]["path"] == GUIDE)
        .unwrap_or_else(|| panic!("the pulled guide must answer: {hits:?}"));
    assert_eq!(
        hit["shared_from"], DOC_REPO,
        "carrying where it came from, since no file here backs it"
    );
    assert_eq!(hit["revision"], payload["revision_hash"]);
}

#[test]
fn an_import_into_an_indexed_checkout_changes_no_local_row_and_no_file() {
    let workspace = tempfile::tempdir().expect("workspace");
    let (_author, _author_root, mut payload) = author(workspace.path());
    // A second engine with its OWN checkout of the same repository.
    let reader_root = docs_tree(workspace.path(), "reader-checkout");
    let reader = Engine::spawn(&[]);
    approve_docs(&reader, &reader_root);
    index_docs_cli(&reader, &reader_root.join("docs/guides"));
    let before_rows = (
        rows(&reader, "documents"),
        rows(&reader, "document_chunks"),
        rows(&reader, "document_fts"),
        rows(&reader, "edges"),
    );
    let before_tree = std::fs::read_to_string(reader_root.join("docs/guides/cloud-sync.md"))
        .expect("read the reader's own copy");
    assert!(before_rows.0 > 0 && before_rows.1 > 0, "{before_rows:?}");
    // The peer is at a different revision of the same document.
    payload["revision_hash"] = serde_json::json!("f".repeat(64));

    let (status, body) = reader.post(
        "/api/v1/sync/replica/import",
        &document_envelope("op-20260923-read0002", &payload),
    );

    assert_eq!(status, 200, "import: {body}");
    assert_eq!(
        (
            rows(&reader, "documents"),
            rows(&reader, "document_chunks"),
            rows(&reader, "document_fts"),
            rows(&reader, "edges"),
        ),
        before_rows,
        "an import writes no local row, and no graph edge for the links it carries"
    );
    assert_eq!(
        std::fs::read_to_string(reader_root.join("docs/guides/cloud-sync.md")).expect("read again"),
        before_tree,
        "and writes no file"
    );
    assert!(
        rows(&reader, "remote_document_link") > 0,
        "the links landed in the pulled cache instead"
    );
    // Both sides hold this document, so search answers once, from the local
    // row — the copy with a file behind it.
    let hits = document_hits(&reader, TERM);
    let for_guide: Vec<&serde_json::Value> = hits
        .iter()
        .filter(|h| h["citation"]["path"] == LOCAL_GUIDE || h["citation"]["path"] == GUIDE)
        .collect();
    assert_eq!(for_guide.len(), 1, "returned once: {hits:?}");
    assert_eq!(
        for_guide[0]["shared_from"],
        serde_json::Value::Null,
        "and it is the local copy"
    );
}

#[test]
fn revoking_the_repository_stops_the_pulled_revision_answering() {
    let workspace = tempfile::tempdir().expect("workspace");
    let (_author, _root, payload) = author(workspace.path());
    let reader = Engine::spawn(&[]);
    approve_docs(&reader, workspace.path());
    let (status, _) = reader.post(
        "/api/v1/sync/replica/import",
        &document_envelope("op-20260923-read0003", &payload),
    );
    assert_eq!(status, 200);
    let answered = document_hits(&reader, TERM);
    assert!(!answered.is_empty(), "it answered while approved");

    // The next policy load no longer lists the repository.
    comemory::store::repository_approval::replace_all(&reader.db(), &[], "2026-09-23T12:00:00Z")
        .expect("revoke");

    assert!(
        document_hits(&reader, TERM).is_empty(),
        "a revoked repository contributes nothing to search"
    );
    assert!(
        rows(&reader, "remote_document_chunk") > 0,
        "though the text is still held — nothing was re-indexed or deleted"
    );

    approve_docs(&reader, workspace.path());
    assert_eq!(
        document_hits(&reader, TERM).len(),
        answered.len(),
        "so reapproval resumes from the same rows"
    );
}
