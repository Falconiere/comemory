#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The shared half of document search, over real extracted documentation:
//! which pulled passages answer, and the two rules that stop them.

use comemory::domains::documents::document::extract::extract;
use comemory::domains::documents::document::{DocumentFormat, ExtractedDocument};
use comemory::domains::documents::share;
use comemory::store::document_fts::{self, HitSource};
use comemory::store::document_share::{self, Share};
use comemory::store::documents::{self, DocumentUpsert};
use comemory::store::remote_document::{self, Chunk, Revision};
use comemory::store::sources::{self, SourceFileUpsert, SourceRootUpsert};
use comemory::store::{connection, migrate, repository_approval};
use rusqlite::Connection;
use tempfile::TempDir;

const REPO: &str = "Falconiere/comemory";
const AT: &str = "2026-09-23T10:00:00Z";

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

fn extracted(path: &str) -> ExtractedDocument {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let body = std::fs::read(&file).unwrap_or_else(|e| panic!("read real {path}: {e}"));
    extract(DocumentFormat::Markdown, &body, "doc").expect("real extraction")
}

/// Put a real pulled revision of `path` in the cache.
fn hold(conn: &Connection, path: &str) -> String {
    let doc = extracted(path);
    let shared_id = share::shared_id(REPO, path);
    let revision = Revision {
        repo: REPO.to_string(),
        shared_id: shared_id.clone(),
        path: path.to_string(),
        title: doc.title.clone(),
        format: "markdown".to_string(),
        revision_hash: "a".repeat(64),
        chunk_count: doc.chunks.len() as i64,
    };
    let chunks: Vec<Chunk> = doc
        .chunks
        .iter()
        .map(|c| Chunk {
            ordinal: c.ordinal as i64,
            heading_path: c.heading_path.join(" > "),
            char_range: (c.char_range.0 as i64, c.char_range.1 as i64),
            line_range: (c.line_range.0 as i64, c.line_range.1 as i64),
            simhash: c.simhash as i64,
            text: c.text.clone(),
        })
        .collect();
    remote_document::replace_revision(conn, &revision, &chunks, &[], AT).expect("hold revision");
    shared_id
}

fn approve(conn: &Connection) {
    repository_approval::replace_all(conn, &[(REPO.to_string(), REPO.to_string())], AT)
        .expect("approve");
}

/// A word the real cloud-sync guide uses and the http-api guide does not.
const GUIDE_TERM: &str = "workspace";

#[test]
fn an_approved_pulled_revision_answers_search() {
    let (_dir, conn) = migrated_db();
    let shared_id = hold(&conn, "docs/guides/cloud-sync.md");
    approve(&conn);

    let hits = document_fts::search(&conn, GUIDE_TERM, 20).expect("search");

    assert!(
        !hits.is_empty(),
        "the real guide uses `{GUIDE_TERM}`, so an empty result would mean the \
         shared half never ran"
    );
    assert!(
        hits.iter().all(|h| h.source
            == HitSource::Shared {
                repo: REPO.to_string(),
                shared_id: shared_id.clone(),
            }),
        "every hit is the pulled revision: {hits:?}"
    );
}

#[test]
fn an_unapproved_repository_answers_nothing_and_resumes_on_reapproval() {
    let (_dir, conn) = migrated_db();
    hold(&conn, "docs/guides/cloud-sync.md");

    // No policy has loaded: the text is held but not shareable back.
    assert!(
        document_fts::search(&conn, GUIDE_TERM, 20)
            .expect("search")
            .is_empty(),
        "an unapproved repository contributes nothing"
    );

    approve(&conn);
    let after = document_fts::search(&conn, GUIDE_TERM, 20).expect("search");
    assert!(!after.is_empty(), "approval alone makes it answer");

    // Revoked: the map is replaced with one that does not list it.
    repository_approval::replace_all(&conn, &[], AT).expect("revoke");
    assert!(
        document_fts::search(&conn, GUIDE_TERM, 20)
            .expect("search")
            .is_empty(),
        "revocation stops it answering, with no re-indexing either way"
    );

    approve(&conn);
    assert_eq!(
        document_fts::search(&conn, GUIDE_TERM, 20).expect("search"),
        after,
        "and reapproval resumes from the same rows"
    );
}

#[test]
fn a_document_the_local_index_holds_is_dropped_from_the_shared_half() {
    let (_dir, conn) = migrated_db();
    let shared_id = hold(&conn, "docs/guides/cloud-sync.md");
    approve(&conn);
    assert!(
        !document_fts::search(&conn, GUIDE_TERM, 20)
            .expect("search")
            .is_empty(),
        "it answered before the mapping existed"
    );

    // The local index claims that shared name, through the real chain a local
    // document hangs off — `document_share.document_id` is a foreign key, so
    // there is no way to assert this with an invented id.
    sources::upsert(
        &conn,
        SourceRootUpsert {
            id: "source-1",
            canonical_path: "/does/not/matter/source-1",
            kind: "dir",
            repo: Some(REPO),
            created_at: AT,
            updated_at: AT,
        },
    )
    .expect("seed source_roots");
    sources::upsert_file(
        &conn,
        SourceFileUpsert {
            id: "file-1",
            source_id: "source-1",
            relative_path: "guides/cloud-sync.md",
            classification: "document",
            size: 0,
            mtime: 0,
            sha256: None,
            status: "indexed",
            error: None,
            created_at: AT,
            updated_at: AT,
        },
    )
    .expect("seed source_files");
    documents::upsert_document(
        &conn,
        DocumentUpsert {
            id: "local-doc-1",
            source_file_id: "file-1",
            title: "Cloud sync",
            repo: Some(REPO),
            revision_hash: "hash",
            created_at: AT,
            updated_at: AT,
        },
    )
    .expect("seed a local document");
    document_share::record(
        &conn,
        &Share {
            document_id: "local-doc-1".to_string(),
            repo: REPO.to_string(),
            shared_id: shared_id.clone(),
            path: "docs/guides/cloud-sync.md".to_string(),
            blocked_reason: None,
        },
        AT,
    )
    .expect("record the mapping");

    let hits = document_fts::search(&conn, GUIDE_TERM, 20).expect("search");

    assert!(
        hits.iter().all(|h| h.source
            != HitSource::Shared {
                repo: REPO.to_string(),
                shared_id: shared_id.clone(),
            }),
        "the pulled half must not answer for a document the local index holds: {hits:?}"
    );
}

#[test]
fn k_bounds_the_union_rather_than_each_half() {
    let (_dir, conn) = migrated_db();
    // Two real pulled documents, both matching, so the union has more
    // passages than `k`.
    hold(&conn, "docs/guides/cloud-sync.md");
    hold(&conn, "docs/guides/replication-e2e.md");
    approve(&conn);
    let all = document_fts::search(&conn, "sync", 100).expect("search");
    assert!(
        all.len() > 3,
        "the corpus must exceed the limit for this to mean anything: {}",
        all.len()
    );

    let limited = document_fts::search(&conn, "sync", 3).expect("search");

    assert_eq!(limited.len(), 3, "exactly `k` across the whole corpus");
    assert_eq!(
        limited,
        all[..3].to_vec(),
        "and they are the best three of the union, not the best three of one \
         side trimmed afterwards"
    );
}

#[test]
fn a_forgotten_revision_answers_nothing() {
    let (_dir, conn) = migrated_db();
    let shared_id = hold(&conn, "docs/guides/cloud-sync.md");
    approve(&conn);
    assert!(
        !document_fts::search(&conn, GUIDE_TERM, 20)
            .expect("search")
            .is_empty()
    );

    remote_document::purge(&conn, REPO, &shared_id).expect("tombstone");

    assert!(
        document_fts::search(&conn, GUIDE_TERM, 20)
            .expect("search")
            .is_empty(),
        "a tombstoned revision leaves no searchable row behind"
    );
}
