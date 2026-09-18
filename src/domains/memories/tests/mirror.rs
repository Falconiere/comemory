#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/domains/memories/mirror.rs` — the one seam every
//! memory writer goes through. Everything here runs against a real migrated
//! `comemory.db` and real Markdown files indexed by the production document
//! writer: the derivation is proven by the rows it lands, and the rollback
//! paths by dropping a real table so a real SQLite error propagates.

use std::fs;
use std::path::PathBuf;

use comemory::domains::documents::document::DocumentFormat;
use comemory::domains::documents::document::writer;
use comemory::domains::documents::source::classify::Classification;
use comemory::domains::documents::source::discover::Candidate;
use comemory::domains::memories::mirror;
use comemory::memory::{Frontmatter, Kind, References, Relations};
use comemory::store::connection;
use comemory::store::sources::{self, SourceRootUpsert};
use rusqlite::{Connection, params};
use tempfile::TempDir;
use time::OffsetDateTime;

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MEMORY_ID: &str = "abc12345";

fn open_db(tmp: &TempDir) -> Connection {
    connection::open(tmp.path().join("comemory.db")).expect("open db")
}

fn sample_fm() -> Frontmatter {
    Frontmatter {
        id: MEMORY_ID.to_string(),
        kind: Kind::Note,
        repo: "qwick".to_string(),
        tags: Vec::new(),
        author: String::new(),
        created: OffsetDateTime::now_utc(),
        quality: 3,
        schema: 1,
        content_hash: "deadbeef".to_string(),
        references: References::default(),
        relations: Relations::default(),
    }
}

/// Mirror `body` for `MEMORY_ID` inside its own transaction, committing only
/// on success — exactly what `save` / `rebuild` / the sync import do.
fn mirror_body(conn: &mut Connection, body: &str) -> comemory::prelude::Result<()> {
    let fm = sample_fm();
    let tx = conn.transaction().expect("tx");
    mirror::insert_row(&tx, &fm, body, "slug-x", "/abs/path.md", &fm.tags)?;
    tx.commit().expect("commit");
    Ok(())
}

fn edge_targets(conn: &Connection, rel: &str) -> Vec<String> {
    conn.prepare("SELECT dst_id FROM edges WHERE src_id = ?1 AND rel = ?2 ORDER BY dst_id")
        .expect("prepare")
        .query_map(params![MEMORY_ID, rel], |r| r.get(0))
        .expect("query")
        .collect::<std::result::Result<Vec<String>, _>>()
        .expect("rows")
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [MEMORY_ID], |r| r.get(0))
        .expect("count")
}

/// Every table `mirror::insert_row` writes, counted for `MEMORY_ID`.
fn row_counts(conn: &Connection) -> (i64, i64, i64, i64) {
    (
        count(conn, "SELECT count(*) FROM memories WHERE id = ?1"),
        count(
            conn,
            "SELECT count(*) FROM memory_tags WHERE memory_id = ?1",
        ),
        count(conn, "SELECT count(*) FROM memory_fts WHERE memory_id = ?1"),
        count(conn, "SELECT count(*) FROM edges WHERE src_id = ?1"),
    )
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

/// Write a real Markdown file under `tmp/<source_id>/<relative>` and index it
/// through the production document writer. Returns its `documents.id`.
fn index_markdown(
    conn: &mut Connection,
    tmp: &TempDir,
    source_id: &str,
    repo: Option<&str>,
    relative: &str,
) -> String {
    let source_root = tmp.path().join(source_id);
    let absolute = source_root.join(relative);
    if let Some(parent) = absolute.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(&absolute, "# Guide\n\nreal markdown fixture\n").expect("write fixture");
    let candidate = Candidate {
        relative_path: PathBuf::from(relative),
        absolute_path: absolute,
        classification: Classification::Document(DocumentFormat::Markdown),
    };
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

#[test]
fn derives_reference_edges_from_the_body() {
    // The v0.2 edge contract: a body naming one file and one (file, symbol)
    // pair lands `references_file` for both paths and `references_symbol`
    // for the qualified symbol, with BARE destination ids (no kind prefix).
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);

    mirror_body(
        &mut conn,
        "Touches qwick-backend:src/db.rs:run_migration and qwick-backend:src/util.rs.",
    )
    .expect("mirror");

    assert_eq!(
        edge_targets(&conn, "references_file"),
        vec![
            "qwick-backend:src/db.rs".to_string(),
            "qwick-backend:src/util.rs".to_string(),
        ],
    );
    assert_eq!(
        edge_targets(&conn, "references_symbol"),
        vec!["qwick-backend:src/db.rs:run_migration".to_string()],
    );
}

#[test]
fn a_body_with_no_reference_lands_no_reference_edge() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);

    mirror_body(&mut conn, "prose with no code citation at all").expect("mirror");

    assert!(edge_targets(&conn, "references_file").is_empty());
    assert!(edge_targets(&conn, "references_symbol").is_empty());
    assert!(edge_targets(&conn, "references_document").is_empty());
    // The row itself still landed — an empty link set is not a failure.
    assert_eq!(
        count(&conn, "SELECT count(*) FROM memories WHERE id = ?1"),
        1
    );
}

#[test]
fn a_mention_of_an_indexed_document_resolves_to_references_document() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    seed_source(&conn, "src-a");
    let doc_id = index_markdown(&mut conn, &tmp, "src-a", Some("qwick"), "docs/guide.md");

    mirror_body(&mut conn, "see qwick:docs/guide.md for the rollout").expect("mirror");

    assert_eq!(edge_targets(&conn, "references_document"), vec![doc_id]);
    // The unresolved `references_file` edge is still emitted: the two are
    // complementary, not alternatives.
    assert_eq!(
        edge_targets(&conn, "references_file"),
        vec!["qwick:docs/guide.md".to_string()],
    );
}

#[test]
fn a_mention_of_an_unindexed_path_derives_no_document_edge() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);

    mirror_body(&mut conn, "see qwick:docs/guide.md for the rollout").expect("mirror");

    assert!(
        edge_targets(&conn, "references_document").is_empty(),
        "nothing to resolve against yet; the document seam completes it later"
    );
}

#[test]
fn an_ambiguous_document_identity_derives_no_edge() {
    // Two live documents at the same `(repo, relative_path)` in different
    // sources: the resolver logs a diagnostic and guesses nothing.
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    seed_source(&conn, "src-a");
    seed_source(&conn, "src-b");
    index_markdown(&mut conn, &tmp, "src-a", Some("qwick"), "docs/guide.md");
    index_markdown(&mut conn, &tmp, "src-b", Some("qwick"), "docs/guide.md");

    mirror_body(&mut conn, "see qwick:docs/guide.md for the rollout").expect("mirror");

    assert!(
        edge_targets(&conn, "references_document").is_empty(),
        "an ambiguous reference stays an unresolved diagnostic, never a guessed edge"
    );
}

#[test]
fn a_derivation_failure_writes_nothing_at_all() {
    // The document lookup runs BEFORE the first write. Dropping the table it
    // reads produces a real SQLite error, and the caller's transaction must
    // come back with no row of any kind for the id.
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    conn.execute("DROP TABLE documents", [])
        .expect("drop documents");

    let err = mirror_body(&mut conn, "see qwick:docs/guide.md for the rollout")
        .expect_err("the derivation must fail");

    assert!(
        err.to_string().contains("documents"),
        "expected the real SQLite error to propagate, got: {err}"
    );
    assert_eq!(row_counts(&conn), (0, 0, 0, 0), "nothing may be written");
}

#[test]
fn a_mid_transaction_store_failure_rolls_the_whole_row_set_back() {
    // `memory_fts` is written after the `memories` upsert, so dropping it
    // fails part-way through the row set. The transaction is dropped without
    // a commit, and every table must read back empty for the id.
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    conn.execute("DROP TABLE memory_fts", [])
        .expect("drop memory_fts");

    let err = mirror_body(&mut conn, "touches qwick-backend:src/db.rs")
        .expect_err("the mirror write must fail");

    assert!(
        err.to_string().contains("memory_fts"),
        "expected the real SQLite error to propagate, got: {err}"
    );
    // `memory_fts` itself is gone, so only the three surviving tables can be
    // read back — each must be empty for the id.
    assert_eq!(
        (
            count(&conn, "SELECT count(*) FROM memories WHERE id = ?1"),
            count(
                &conn,
                "SELECT count(*) FROM memory_tags WHERE memory_id = ?1"
            ),
            count(&conn, "SELECT count(*) FROM edges WHERE src_id = ?1"),
        ),
        (0, 0, 0),
        "a partial mirror must leave no trace"
    );
}

#[test]
fn a_repeat_mirror_is_idempotent_for_the_derived_edges() {
    // Every re-mirror seam (metadata PATCH, restore, refresh-refs, rebuild)
    // replays this path over an existing id. The outgoing wipe plus the
    // re-derivation must leave exactly one edge per target, not a duplicate.
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let body = "Touches qwick-backend:src/db.rs:run_migration.";

    mirror_body(&mut conn, body).expect("first mirror");
    mirror_body(&mut conn, body).expect("second mirror");

    assert_eq!(
        edge_targets(&conn, "references_file"),
        vec!["qwick-backend:src/db.rs".to_string()],
    );
    assert_eq!(
        edge_targets(&conn, "references_symbol"),
        vec!["qwick-backend:src/db.rs:run_migration".to_string()],
    );
}

#[test]
fn a_re_mirror_with_a_changed_body_drops_the_stale_reference_edge() {
    // The outgoing-edge wipe is what makes a removed citation actually
    // disappear; without it the `edges` table would only ever grow.
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);

    mirror_body(&mut conn, "Touches qwick-backend:src/db.rs.").expect("first mirror");
    mirror_body(&mut conn, "Now touches qwick-backend:src/util.rs.").expect("second mirror");

    assert_eq!(
        edge_targets(&conn, "references_file"),
        vec!["qwick-backend:src/util.rs".to_string()],
        "the stale citation must be gone, not accumulated"
    );
}
