#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/graph/doc_link.rs`: the deterministic
//! `member_of_source` / `references_document` link deriver, exercised
//! through the real seams (`document::writer::update_file`,
//! `store::memory_row::insert`) against a real migrated `comemory.db`,
//! never by calling `graph::doc_link` functions directly.

use std::fs;
use std::path::PathBuf;

use comemory::document::DocumentFormat;
use comemory::document::writer;
use comemory::memory::{Frontmatter, Kind, References, Relations};
use comemory::retrieval::graph_route::ALLOWED_RELS;
use comemory::source::classify::Classification;
use comemory::source::discover::Candidate;
use comemory::store::sources::SourceRootUpsert;
use comemory::store::{connection, memory_row, sources};
use rusqlite::{Connection, params};
use tempfile::TempDir;
use time::OffsetDateTime;

const MAX_BYTES: u64 = 16 * 1024 * 1024;

fn open_db(tmp: &TempDir) -> Connection {
    connection::open(tmp.path().join("comemory.db")).expect("open db")
}

/// Register a `source_roots` row so `source_files` FK inserts succeed.
fn seed_source(conn: &Connection, source_id: &str) {
    sources::upsert(
        conn,
        SourceRootUpsert {
            id: source_id,
            canonical_path: &format!("/does/not/matter/{source_id}"),
            kind: "dir",
            repo: None,
            created_at: "2026-01-01T00:00:00.000000000Z",
            updated_at: "2026-01-01T00:00:00.000000000Z",
        },
    )
    .expect("seed source_roots row");
}

/// Write `content` to `tmp/<relative>` and index it under `source_id` as
/// Markdown, returning the resulting `documents.id`. Panics on anything
/// but `Indexed` — every fixture here is a fresh, valid Markdown file.
fn index_markdown(
    conn: &mut Connection,
    tmp: &TempDir,
    source_id: &str,
    repo: Option<&str>,
    relative: &str,
    content: &str,
) -> String {
    // Namespaced by `source_id` so two sources can each claim the same
    // `relative` path without colliding on disk.
    let source_root = tmp.path().join(source_id);
    let absolute = source_root.join(relative);
    if let Some(parent) = absolute.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(&absolute, content).expect("write fixture");
    let candidate = Candidate {
        relative_path: PathBuf::from(relative),
        absolute_path: absolute,
        classification: Classification::Document(DocumentFormat::Markdown),
    };
    // `source_root` itself (not `absolute`'s parent) is the writer's
    // TOCTOU boundary — `relative` may nest under a subdirectory
    // (e.g. `sub/guide.md`), which is still inside the source, not the
    // whole boundary.
    let canonical_root = fs::canonicalize(&source_root).expect("canonicalize source root");
    let outcome = writer::update_file(
        conn,
        source_id,
        repo,
        &candidate,
        &canonical_root,
        MAX_BYTES,
    )
    .expect("update_file");
    match outcome {
        writer::UpdateOutcome::Indexed { document_id } => document_id,
        other => panic!("expected Indexed, got {other:?}"),
    }
}

/// Save a real memory (through the same `memory_row::insert` seam
/// `cli::save` uses) whose body is exactly `body`, inside its own
/// transaction. Returns the memory id.
fn save_memory(conn: &mut Connection, id: &str, repo: &str, body: &str) -> String {
    let fm = Frontmatter {
        id: id.to_string(),
        kind: Kind::Note,
        repo: repo.to_string(),
        tags: Vec::new(),
        author: String::new(),
        created: OffsetDateTime::now_utc(),
        quality: 3,
        schema: 1,
        content_hash: "deadbeef".to_string(),
        references: References::default(),
        relations: Relations::default(),
    };
    let tx = conn.transaction().expect("tx");
    memory_row::insert(&tx, &fm, body, "slug", "/abs/path.md", &[]).expect("insert memory");
    tx.commit().expect("commit");
    id.to_string()
}

fn references_document_edge(conn: &Connection, src_kind: &str, src_id: &str, dst_id: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM edges \
          WHERE rel = 'references_document' AND src_kind = ?1 AND src_id = ?2 \
            AND dst_kind = 'document' AND dst_id = ?3",
        params![src_kind, src_id, dst_id],
        |r| r.get(0),
    )
    .expect("count references_document edges")
}

#[test]
fn document_indexed_then_memory_saved_resolves_references_document() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    seed_source(&conn, "src-a");

    let doc_id = index_markdown(
        &mut conn,
        &tmp,
        "src-a",
        Some("demo"),
        "guide.md",
        "# Guide\n\nsetup steps.\n",
    );
    let mem_id = save_memory(
        &mut conn,
        "aaaa1111",
        "demo",
        "see `demo:guide.md` for setup",
    );

    assert_eq!(
        references_document_edge(&conn, "memory", &mem_id, &doc_id),
        1,
        "memory saved after the document must resolve the reference"
    );
}

#[test]
fn memory_saved_then_document_indexed_resolves_references_document() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    seed_source(&conn, "src-b");

    let mem_id = save_memory(
        &mut conn,
        "bbbb2222",
        "demo",
        "see `demo:guide.md` for setup",
    );
    let doc_id = index_markdown(
        &mut conn,
        &tmp,
        "src-b",
        Some("demo"),
        "guide.md",
        "# Guide\n\nsetup steps.\n",
    );

    assert_eq!(
        references_document_edge(&conn, "memory", &mem_id, &doc_id),
        1,
        "document indexed after the memory must resolve the reference (order-independence)"
    );
}

#[test]
fn member_of_source_edge_exists_and_is_excluded_from_the_graph_walk() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    seed_source(&conn, "src-c");

    index_markdown(
        &mut conn,
        &tmp,
        "src-c",
        None,
        "notes.md",
        "# Notes\n\nbody.\n",
    );

    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM edges WHERE rel = 'member_of_source' \
               AND src_kind = 'file' AND dst_kind = 'source' AND dst_id = 'src-c'",
            [],
            |r| r.get(0),
        )
        .expect("count member_of_source");
    assert_eq!(
        n, 1,
        "indexing a document must derive its member_of_source edge"
    );

    assert!(
        !ALLOWED_RELS.contains(&"member_of_source"),
        "member_of_source must stay outside the graph-expansion walk"
    );
    assert!(
        !ALLOWED_RELS.contains(&"references_document"),
        "references_document must stay outside the graph-expansion walk"
    );
}

#[test]
fn markdown_link_resolves_uniquely_within_the_same_source() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    seed_source(&conn, "src-d");

    let target_id = index_markdown(
        &mut conn,
        &tmp,
        "src-d",
        None,
        "sub/guide.md",
        "# Guide\n\ndetails.\n",
    );
    let index_id = index_markdown(
        &mut conn,
        &tmp,
        "src-d",
        None,
        "index.md",
        "# Index\n\nsee [the guide](sub/guide.md) for details.\n",
    );

    assert_eq!(
        references_document_edge(&conn, "document", &index_id, &target_id),
        1,
        "a uniquely-resolving relative Markdown link must derive a document edge"
    );
}

#[test]
fn ambiguous_markdown_link_target_derives_no_edge() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    seed_source(&conn, "src-e1");
    seed_source(&conn, "src-e2");
    seed_source(&conn, "src-e3");

    // Two different sources under the same repo label both index a
    // document at the identical relative path `dup.md`.
    let dup_1 = index_markdown(
        &mut conn,
        &tmp,
        "src-e2",
        Some("shared"),
        "dup.md",
        "# One\n",
    );
    let dup_2 = index_markdown(
        &mut conn,
        &tmp,
        "src-e3",
        Some("shared"),
        "dup.md",
        "# Two\n",
    );

    // A third source's document links to that same relative path — not
    // present in its OWN source, so resolution falls back to the
    // repo-wide lookup, which now matches two documents.
    let linker_id = index_markdown(
        &mut conn,
        &tmp,
        "src-e1",
        Some("shared"),
        "linker.md",
        "see [dup](dup.md)\n",
    );

    assert_eq!(
        references_document_edge(&conn, "document", &linker_id, &dup_1),
        0,
        "ambiguous repo-scoped match must not derive an edge to either candidate"
    );
    assert_eq!(
        references_document_edge(&conn, "document", &linker_id, &dup_2),
        0
    );
}
