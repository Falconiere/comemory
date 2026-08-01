//! Test mirror for `src/store/documents.rs`: `documents` +
//! `document_chunks` row CRUD, exercised against real extracted content
//! from the checked-in fixtures (`tests/common/fixtures/docs/`) rather
//! than hand-typed placeholder rows.

use comemory::document::extract::extract;
use comemory::document::{DocumentFormat, ExtractedDocument};
use comemory::store::connection;
use comemory::store::documents::{self, ChunkRow, DocumentUpsert};
use comemory::store::sources::{self, SourceFileUpsert, SourceRootUpsert};
use rusqlite::Connection;
use tempfile::TempDir;

const GUIDE_MD: &[u8] = include_bytes!("common/fixtures/docs/guide.md");
const CHANGELOG_TXT: &[u8] = include_bytes!("common/fixtures/docs/changelog.txt");

const T1: &str = "2026-01-01T00:00:00.000000000Z";
const T2: &str = "2026-01-02T00:00:00.000000000Z";

fn open_db() -> (Connection, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");
    (conn, tmp)
}

/// Seed one `source_roots` row (idempotent — `ON CONFLICT` upsert) and
/// one `source_files` row under it, so a `documents` row can legally FK
/// to `file_id`: `documents.source_file_id` is `NOT NULL UNIQUE
/// REFERENCES source_files(id)`. `canonical_path` is derived from
/// `source_id` since `source_roots.canonical_path` is itself `UNIQUE` —
/// two distinct sources need two distinct paths.
fn seed_source_file(conn: &Connection, source_id: &str, file_id: &str, relative_path: &str) {
    sources::upsert(
        conn,
        SourceRootUpsert {
            id: source_id,
            canonical_path: &format!("/does/not/matter/{source_id}"),
            kind: "dir",
            repo: None,
            created_at: T1,
            updated_at: T1,
        },
    )
    .expect("seed source_roots row");
    sources::upsert_file(
        conn,
        SourceFileUpsert {
            id: file_id,
            source_id,
            relative_path,
            classification: "document",
            size: 0,
            mtime: 0,
            sha256: None,
            status: "indexed",
            error: None,
            created_at: T1,
            updated_at: T1,
        },
    )
    .expect("seed source_files row");
}

/// [`seed_source_file`] plus a `documents` row over it, so
/// `document_chunks`/[`documents::replace_chunks`] rows (FK-referencing
/// `documents(id)`) resolve.
fn seed_document(
    conn: &Connection,
    source_id: &str,
    file_id: &str,
    relative_path: &str,
    document_id: &str,
) {
    seed_source_file(conn, source_id, file_id, relative_path);
    documents::upsert_document(
        conn,
        DocumentUpsert {
            id: document_id,
            source_file_id: file_id,
            title: "T",
            repo: None,
            revision_hash: "hash",
            created_at: T1,
            updated_at: T1,
        },
    )
    .expect("seed documents row");
}

/// Real extraction of `guide.md` — multi-chunk Markdown content, not a
/// hand-typed placeholder.
fn guide() -> ExtractedDocument {
    extract(DocumentFormat::Markdown, GUIDE_MD, "guide").expect("extract guide.md")
}

fn changelog() -> ExtractedDocument {
    extract(DocumentFormat::Txt, CHANGELOG_TXT, "changelog").expect("extract changelog.txt")
}

/// `Chunk` -> `ChunkRow` for real extracted chunks: every store-facing
/// field (ordinal/ranges/simhash/text) carries its real extracted value.
/// `heading_path` is the chunk's deepest breadcrumb entry rather than
/// `writer::write_chunks`'s `" > "`-joined string — no test here asserts
/// on it, and a single borrowed `&str` avoids an extra owned buffer.
fn to_chunk_rows(extracted: &ExtractedDocument) -> Vec<ChunkRow<'_>> {
    extracted
        .chunks
        .iter()
        .map(|c| ChunkRow {
            ordinal: c.ordinal as i64,
            heading_path: c.heading_path.last().map(String::as_str).unwrap_or(""),
            char_range: (c.char_range.0 as i64, c.char_range.1 as i64),
            line_range: (c.line_range.0 as i64, c.line_range.1 as i64),
            simhash: c.simhash as i64,
            text: c.text.as_str(),
        })
        .collect()
}

#[test]
fn upsert_then_get_roundtrips_fields() {
    let (conn, _tmp) = open_db();
    seed_source_file(&conn, "src-a", "file-a", "guide.md");
    let extracted = guide();

    documents::upsert_document(
        &conn,
        DocumentUpsert {
            id: "doc-a",
            source_file_id: "file-a",
            title: &extracted.title,
            repo: Some("comemory"),
            revision_hash: "hash-1",
            created_at: T1,
            updated_at: T1,
        },
    )
    .expect("upsert");

    let row = documents::get_document(&conn, "doc-a")
        .expect("get")
        .expect("row exists");
    assert_eq!(row.id, "doc-a");
    assert_eq!(row.source_file_id, "file-a");
    assert_eq!(row.title, extracted.title);
    assert_eq!(row.repo.as_deref(), Some("comemory"));
    assert_eq!(row.revision_hash, "hash-1");
    assert_eq!(row.created_at, T1);
}

#[test]
fn get_document_returns_none_for_unknown_id() {
    let (conn, _tmp) = open_db();
    assert!(
        documents::get_document(&conn, "does-not-exist")
            .expect("get")
            .is_none()
    );
}

#[test]
fn upsert_on_conflict_updates_mutable_fields_and_keeps_created_at() {
    let (conn, _tmp) = open_db();
    seed_source_file(&conn, "src-a", "file-a", "guide.md");

    documents::upsert_document(
        &conn,
        DocumentUpsert {
            id: "doc-a",
            source_file_id: "file-a",
            title: "First",
            repo: None,
            revision_hash: "hash-1",
            created_at: T1,
            updated_at: T1,
        },
    )
    .expect("first upsert");

    documents::upsert_document(
        &conn,
        DocumentUpsert {
            id: "doc-a",
            source_file_id: "file-a",
            title: "Second",
            repo: Some("comemory"),
            revision_hash: "hash-2",
            created_at: "2099-01-01T00:00:00.000000000Z",
            updated_at: T2,
        },
    )
    .expect("second upsert");

    let row = documents::get_document(&conn, "doc-a")
        .expect("get")
        .expect("row exists");
    assert_eq!(row.title, "Second", "title must refresh");
    assert_eq!(row.repo.as_deref(), Some("comemory"), "repo must refresh");
    assert_eq!(row.revision_hash, "hash-2", "revision_hash must refresh");
    assert_eq!(row.updated_at, T2, "updated_at must refresh");
    assert_eq!(
        row.created_at, T1,
        "created_at must survive a conflicting upsert untouched"
    );
}

#[test]
fn delete_document_removes_row_and_is_noop_when_missing() {
    let (conn, _tmp) = open_db();
    seed_source_file(&conn, "src-a", "file-a", "guide.md");
    documents::upsert_document(
        &conn,
        DocumentUpsert {
            id: "doc-a",
            source_file_id: "file-a",
            title: "T",
            repo: None,
            revision_hash: "hash-1",
            created_at: T1,
            updated_at: T1,
        },
    )
    .expect("upsert");

    documents::delete_document(&conn, "doc-a").expect("delete");
    assert!(
        documents::get_document(&conn, "doc-a")
            .expect("get")
            .is_none()
    );

    // Deleting an id with no row is a no-op, not an error.
    documents::delete_document(&conn, "doc-a").expect("delete again");
    documents::delete_document(&conn, "never-existed").expect("delete unknown");
}

#[test]
fn replace_chunks_deletes_previous_set_and_inserts_new_one() {
    let (conn, _tmp) = open_db();
    seed_document(&conn, "src-a", "file-a", "guide.md", "doc-a");
    let guide_extracted = guide();
    documents::replace_chunks(&conn, "doc-a", &to_chunk_rows(&guide_extracted)).expect("first");

    let guide_chunk_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM document_chunks WHERE document_id = ?1",
            ["doc-a"],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(guide_chunk_count, guide_extracted.chunks.len() as i64);
    assert!(guide_chunk_count > 1, "guide.md has multiple chunks");

    let changelog_extracted = changelog();
    documents::replace_chunks(&conn, "doc-a", &to_chunk_rows(&changelog_extracted))
        .expect("replace");

    let after_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM document_chunks WHERE document_id = ?1",
            ["doc-a"],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(
        after_count,
        changelog_extracted.chunks.len() as i64,
        "replace must delete the previous set, not append"
    );
}

#[test]
fn get_chunk_returns_citation_fields_and_none_for_missing_ordinal() {
    let (conn, _tmp) = open_db();
    seed_document(&conn, "src-a", "file-a", "guide.md", "doc-a");
    let extracted = guide();
    documents::replace_chunks(&conn, "doc-a", &to_chunk_rows(&extracted)).expect("replace");

    let first_chunk = &extracted.chunks[0];
    let citation = documents::get_chunk(&conn, "doc-a", 0)
        .expect("get_chunk")
        .expect("chunk 0 exists");
    assert_eq!(citation.text, first_chunk.text);
    assert_eq!(
        citation.line_range,
        (
            first_chunk.line_range.0 as i64,
            first_chunk.line_range.1 as i64
        )
    );

    let missing_ordinal = extracted.chunks.len() as i64 + 100;
    assert!(
        documents::get_chunk(&conn, "doc-a", missing_ordinal)
            .expect("get_chunk")
            .is_none()
    );
}

#[test]
fn get_chunk_returns_none_for_missing_document() {
    let (conn, _tmp) = open_db();
    seed_document(&conn, "src-a", "file-a", "guide.md", "doc-a");
    documents::replace_chunks(&conn, "doc-a", &to_chunk_rows(&guide())).expect("replace");

    assert!(
        documents::get_chunk(&conn, "no-such-doc", 0)
            .expect("get_chunk")
            .is_none()
    );
}

#[test]
fn get_document_path_joins_through_source_files_and_none_when_missing() {
    let (conn, _tmp) = open_db();
    seed_source_file(&conn, "src-a", "file-a", "docs/guide.md");
    documents::upsert_document(
        &conn,
        DocumentUpsert {
            id: "doc-a",
            source_file_id: "file-a",
            title: "T",
            repo: None,
            revision_hash: "hash-1",
            created_at: T1,
            updated_at: T1,
        },
    )
    .expect("upsert");

    let path = documents::get_document_path(&conn, "doc-a")
        .expect("get_document_path")
        .expect("path exists");
    assert_eq!(path, "docs/guide.md");

    assert!(
        documents::get_document_path(&conn, "no-such-doc")
            .expect("get_document_path")
            .is_none()
    );
}

#[test]
fn document_ids_for_source_lists_only_that_sources_documents() {
    let (conn, _tmp) = open_db();
    seed_source_file(&conn, "src-a", "file-a1", "guide.md");
    seed_source_file(&conn, "src-a", "file-a2", "changelog.txt");
    seed_source_file(&conn, "src-b", "file-b1", "guide.md");

    for (id, file_id) in [
        ("doc-a1", "file-a1"),
        ("doc-a2", "file-a2"),
        ("doc-b1", "file-b1"),
    ] {
        documents::upsert_document(
            &conn,
            DocumentUpsert {
                id,
                source_file_id: file_id,
                title: "T",
                repo: None,
                revision_hash: "hash",
                created_at: T1,
                updated_at: T1,
            },
        )
        .expect("upsert");
    }

    let mut ids_a = documents::document_ids_for_source(&conn, "src-a").expect("list src-a");
    ids_a.sort();
    assert_eq!(ids_a, vec!["doc-a1".to_string(), "doc-a2".to_string()]);

    let ids_b = documents::document_ids_for_source(&conn, "src-b").expect("list src-b");
    assert_eq!(ids_b, vec!["doc-b1".to_string()]);

    let ids_none = documents::document_ids_for_source(&conn, "src-nonexistent").expect("list");
    assert!(ids_none.is_empty());
}
