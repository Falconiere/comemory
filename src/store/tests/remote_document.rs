#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `remote_document` against a real migrated database, over real extracted
//! content: what replacing a revision must leave behind, and what it must not.
//!
//! The revisions are produced by running this repository's own documentation
//! through the real extractor, so the chunk counts, ranges and heading paths
//! are whatever the shipped code actually produces.

use comemory::domains::documents::document::extract::extract;
use comemory::domains::documents::document::{DocumentFormat, ExtractedDocument};
use comemory::store::remote_document::{self, Chunk, Link, Revision};
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

const REPO: &str = "Falconiere/comemory";
const AT: &str = "2026-09-23T10:00:00Z";
const LATER: &str = "2026-09-23T11:00:00Z";

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

/// A real repository document, extracted by the shipped extractor.
fn extracted(path: &str) -> ExtractedDocument {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let body = std::fs::read(&file).unwrap_or_else(|e| panic!("read real {path}: {e}"));
    extract(DocumentFormat::Markdown, &body, "doc").expect("real extraction")
}

fn chunks_of(doc: &ExtractedDocument) -> Vec<Chunk> {
    doc.chunks
        .iter()
        .map(|c| Chunk {
            ordinal: c.ordinal as i64,
            heading_path: c.heading_path.join(" > "),
            char_range: (c.char_range.0 as i64, c.char_range.1 as i64),
            line_range: (c.line_range.0 as i64, c.line_range.1 as i64),
            simhash: c.simhash as i64,
            text: c.text.clone(),
        })
        .collect()
}

fn revision_of(doc: &ExtractedDocument, path: &str, hash: &str) -> Revision {
    Revision {
        repo: REPO.to_string(),
        shared_id: comemory::domains::documents::share::shared_id(REPO, path),
        path: path.to_string(),
        title: doc.title.clone(),
        format: "markdown".to_string(),
        revision_hash: hash.to_string(),
        chunk_count: doc.chunks.len() as i64,
    }
}

fn count(conn: &Connection, table: &str, shared_id: &str) -> i64 {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE shared_id = ?1"),
        [shared_id],
        |r| r.get(0),
    )
    .expect("count")
}

#[test]
fn a_revision_lands_whole_and_reads_back_as_extracted() {
    let (_dir, conn) = migrated_db();
    let doc = extracted("docs/guides/cloud-sync.md");
    let path = "docs/guides/cloud-sync.md";
    let revision = revision_of(&doc, path, &"a".repeat(64));
    let chunks = chunks_of(&doc);
    let links = vec![Link {
        ordinal: 0,
        target: "docs/guides/http-api.md".to_string(),
    }];

    remote_document::replace_revision(&conn, &revision, &chunks, &links, AT)
        .expect("replace revision");

    assert!(!chunks.is_empty(), "the real document really has passages");
    assert_eq!(
        remote_document::revision(&conn, REPO, &revision.shared_id).expect("read"),
        Some(revision.clone())
    );
    assert_eq!(
        remote_document::all::<Chunk>(&conn, REPO, &revision.shared_id).expect("read chunks"),
        chunks,
        "every range, heading and passage survives the round trip"
    );
    assert_eq!(
        remote_document::all::<Link>(&conn, REPO, &revision.shared_id).expect("read links"),
        links
    );
    assert_eq!(
        count(&conn, "remote_document_fts", &revision.shared_id),
        chunks.len() as i64,
        "one searchable row per passage"
    );
}

#[test]
fn a_shorter_revision_leaves_no_passage_of_the_longer_one_behind() {
    let (_dir, conn) = migrated_db();
    let doc = extracted("docs/guides/cloud-sync.md");
    let path = "docs/guides/cloud-sync.md";
    let long = revision_of(&doc, path, &"a".repeat(64));
    let long_chunks = chunks_of(&doc);
    assert!(
        long_chunks.len() > 1,
        "the fixture must have more than one passage for this to mean anything"
    );
    remote_document::replace_revision(
        &conn,
        &long,
        &long_chunks,
        &[Link {
            ordinal: 0,
            target: "docs/guides/http-api.md".to_string(),
        }],
        AT,
    )
    .expect("first");

    // The sender edited the document down to its first passage.
    let mut short = long.clone();
    short.revision_hash = "b".repeat(64);
    short.chunk_count = 1;
    remote_document::replace_revision(&conn, &short, &long_chunks[..1], &[], LATER)
        .expect("second");

    assert_eq!(
        remote_document::revision(&conn, REPO, &short.shared_id)
            .expect("read")
            .expect("row")
            .revision_hash,
        "b".repeat(64),
        "the later revision is the one held"
    );
    assert_eq!(
        remote_document::all::<Chunk>(&conn, REPO, &short.shared_id)
            .expect("read")
            .len(),
        1,
        "the passages the sender dropped are gone, not merely shadowed"
    );
    assert_eq!(
        count(&conn, "remote_document_fts", &short.shared_id),
        1,
        "and search cannot still match them"
    );
    assert!(
        remote_document::all::<Link>(&conn, REPO, &short.shared_id)
            .expect("read")
            .is_empty(),
        "nor can a link the sender removed still resolve"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM remote_document", [], |r| r
            .get::<_, i64>(0))
            .expect("count"),
        1,
        "one row per document, replaced rather than accumulated"
    );
}

#[test]
fn purging_one_document_leaves_another_untouched() {
    let (_dir, conn) = migrated_db();
    let kept_doc = extracted("docs/guides/cloud-sync.md");
    let gone_doc = extracted("docs/guides/http-api.md");
    let kept = revision_of(&kept_doc, "docs/guides/cloud-sync.md", &"a".repeat(64));
    let gone = revision_of(&gone_doc, "docs/guides/http-api.md", &"b".repeat(64));
    remote_document::replace_revision(&conn, &kept, &chunks_of(&kept_doc), &[], AT)
        .expect("write kept");
    remote_document::replace_revision(&conn, &gone, &chunks_of(&gone_doc), &[], AT)
        .expect("write gone");
    assert!(
        count(&conn, "remote_document_chunk", &gone.shared_id) > 0,
        "it was really there, so its absence below means something"
    );

    remote_document::purge(&conn, REPO, &gone.shared_id).expect("purge");

    assert_eq!(
        remote_document::revision(&conn, REPO, &gone.shared_id).expect("read"),
        None
    );
    assert_eq!(count(&conn, "remote_document_chunk", &gone.shared_id), 0);
    assert_eq!(count(&conn, "remote_document_fts", &gone.shared_id), 0);
    assert_eq!(
        remote_document::revision(&conn, REPO, &kept.shared_id).expect("read"),
        Some(kept.clone()),
        "the other document is untouched"
    );
    assert!(count(&conn, "remote_document_chunk", &kept.shared_id) > 0);
}

#[test]
fn the_same_path_under_another_repository_is_a_different_document() {
    let (_dir, conn) = migrated_db();
    let doc = extracted("docs/guides/cloud-sync.md");
    let path = "docs/guides/cloud-sync.md";
    let ours = revision_of(&doc, path, &"a".repeat(64));
    let mut theirs = ours.clone();
    theirs.repo = "Falconiere/other".to_string();
    theirs.shared_id = comemory::domains::documents::share::shared_id("Falconiere/other", path);

    remote_document::replace_revision(&conn, &ours, &chunks_of(&doc), &[], AT).expect("ours");
    remote_document::replace_revision(&conn, &theirs, &chunks_of(&doc), &[], AT).expect("theirs");

    assert_ne!(ours.shared_id, theirs.shared_id);
    assert_eq!(
        remote_document::revision(&conn, REPO, &ours.shared_id)
            .expect("read")
            .expect("row")
            .repo,
        REPO
    );
    assert_eq!(
        remote_document::revision(&conn, "Falconiere/other", &theirs.shared_id)
            .expect("read")
            .expect("row")
            .repo,
        "Falconiere/other",
        "two repositories holding the same path do not collide"
    );
}
