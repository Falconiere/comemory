#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`erase_rows`] against a real migrated `comemory.db` populated through the
//! real writers — `domains::memories::save::run`, the document writer, the
//! pulled-document cache — and real WAL files on disk: what each erase step
//! removes, what it keeps, and that `secure_delete` plus the truncating
//! checkpoint leave no byte of the erased text in the database file.

use std::fs;
use std::path::Path;

use comemory::config::{Config, Paths};
use comemory::domains::memories::Kind;
use comemory::store::erase_rows::{self, FtsIndex, LocalDocument};
use comemory::store::remote_document::{self, Chunk, Revision};
use comemory::store::{connection, replica_read};
use comemory::utilities::context::Ctx;
use rusqlite::Connection;
use tempfile::TempDir;

const NOW: &str = "2026-09-25T10:00:00Z";

fn open(home: &Path) -> (Paths, Config, Connection) {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure_dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

/// Save one memory through the real core, with a caller-supplied title so
/// the `save` activity summary records one.
fn save(paths: &Paths, cfg: &Config, conn: &mut Connection, body: &str, title: &str) -> String {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    crate::domains::memories::save::run(
        &mut ctx,
        crate::domains::memories::save::Request {
            body: body.to_string(),
            title: Some(title.to_string()),
            kind: Kind::Note,
            repo: "demo".to_string(),
            tags: vec!["erase".to_string()],
            author: "tester".to_string(),
            quality: 3,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        },
        false,
        None,
    )
    .expect("save")
    .id
}

fn count(conn: &Connection, sql: &str, key: &str) -> i64 {
    conn.query_row(sql, [key], |r| r.get(0)).expect("count")
}

/// Whether `needle` appears anywhere in the bytes of `path`.
fn file_holds(path: &Path, needle: &str) -> bool {
    let bytes = fs::read(path).unwrap_or_default();
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[test]
fn memory_state_reads_a_live_and_a_trashed_row() {
    let home = TempDir::new().expect("tempdir");
    let (paths, cfg, mut conn) = open(home.path());
    let id = save(
        &paths,
        &cfg,
        &mut conn,
        "a memory whose state is read back",
        "State",
    );

    let live = erase_rows::memory_state(&conn, &id)
        .expect("state")
        .expect("row");
    assert!(live.live);
    assert_eq!(live.repo, "demo");
    assert_eq!(live.content_hash.len(), 64);

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    crate::domains::memories::delete::run(&mut ctx, &id).expect("delete");
    let trashed = erase_rows::memory_state(&conn, &id)
        .expect("state")
        .expect("row");
    assert!(!trashed.live, "a trashed row reads as not live");
    assert_eq!(
        erase_rows::memory_state(&conn, "0badc0de").expect("state"),
        None
    );
}

#[test]
fn erase_memory_removes_every_row_keyed_by_it_and_keeps_the_journal() {
    let home = TempDir::new().expect("tempdir");
    let (paths, cfg, mut conn) = open(home.path());
    let id = save(
        &paths,
        &cfg,
        &mut conn,
        "Erase must reach every mirror row: quokkafjord lives only here.",
        "Quokkafjord title",
    );
    let feed_before = replica_read::page(&conn, 0, 50, None).expect("page").len();

    let tx = conn.transaction().expect("begin");
    erase_rows::erase_memory(&tx, &id, NOW).expect("erase rows");
    tx.commit().expect("commit");

    for (table, sql) in [
        ("memories", "SELECT COUNT(*) FROM memories WHERE id = ?1"),
        (
            "memory_tags",
            "SELECT COUNT(*) FROM memory_tags WHERE memory_id = ?1",
        ),
        (
            "memory_fts",
            "SELECT COUNT(*) FROM memory_fts WHERE memory_id = ?1",
        ),
        (
            "memory_write_intent",
            "SELECT COUNT(*) FROM memory_write_intent WHERE entity_key = ?1",
        ),
        (
            "memory_needs_embedding",
            "SELECT COUNT(*) FROM memory_needs_embedding WHERE memory_id = ?1",
        ),
        (
            "edges",
            "SELECT COUNT(*) FROM edges WHERE (src_kind = 'memory' AND src_id = ?1) \
                OR (dst_kind = 'memory' AND dst_id = ?1)",
        ),
    ] {
        assert_eq!(count(&conn, sql, &id), 0, "{table} still holds {id}");
    }
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM memory_substring WHERE memory_substring MATCH ?1",
            "quokkafjord"
        ),
        0,
        "the trigram index no longer finds the text"
    );
    let summary: String = conn
        .query_row(
            "SELECT summary FROM activity_log WHERE command = 'save' \
              AND json_extract(summary, '$.id') = ?1",
            [&id],
            |r| r.get(0),
        )
        .expect("the save run is still recorded");
    let summary: serde_json::Value = serde_json::from_str(&summary).expect("json");
    assert_eq!(
        summary["id"],
        serde_json::json!(id),
        "the run keeps naming its id"
    );
    assert!(
        summary["title"].is_null(),
        "but its title is gone: {summary}"
    );
    assert_eq!(
        replica_read::page(&conn, 0, 50, None).expect("page").len(),
        feed_before,
        "no feed row is removed"
    );
    assert!(
        replica_read::revision(&conn, "memory", &id)
            .expect("revision")
            .is_some(),
        "the revision stays"
    );
}

#[test]
fn secure_delete_leaves_no_byte_of_an_erased_row_in_the_database_file() {
    let home = TempDir::new().expect("tempdir");
    let (paths, cfg, mut conn) = open(home.path());
    let token = "zyxwvuTOKENqpo7731";
    let id = save(
        &paths,
        &cfg,
        &mut conn,
        &format!("A body carrying {token} and nothing else of note."),
        "Plain",
    );
    // A second write that rewrites the same row, so the page also holds a
    // stale copy of the cell an ordinary delete would leave behind.
    conn.execute(
        "UPDATE memories SET rank_score = 0.5, last_accessed = ?2 WHERE id = ?1",
        [&id, NOW],
    )
    .expect("rewrite the row");

    erase_rows::with_secure_delete(&mut conn, |conn| {
        let tx = conn.transaction()?;
        comemory::store::replica_redaction::redact(
            &tx,
            comemory::store::replica_redaction::Reach::Entity("memory", &id),
            NOW,
        )?;
        erase_rows::erase_memory(&tx, &id, NOW)?;
        tx.commit()?;
        for index in [FtsIndex::Memories, FtsIndex::MemorySubstring] {
            erase_rows::optimize(conn, index)?;
        }
        Ok(())
    })
    .expect("erase");
    assert!(
        erase_rows::truncate_wal(&conn, std::time::Duration::from_secs(5)).expect("checkpoint"),
        "no reader holds the WAL"
    );

    let wal = paths.db_path().with_extension("db-wal");
    assert_eq!(fs::metadata(&wal).map_or(0, |m| m.len()), 0);
    assert!(
        !file_holds(&paths.db_path(), token),
        "the erased text survives somewhere in comemory.db"
    );
}

#[test]
fn with_secure_delete_restores_the_prior_setting_even_on_failure() {
    let home = TempDir::new().expect("tempdir");
    let (_paths, _cfg, mut conn) = open(home.path());
    let setting = |conn: &Connection| -> i64 {
        conn.query_row("PRAGMA secure_delete", [], |r| r.get(0))
            .expect("pragma")
    };
    conn.execute_batch("PRAGMA secure_delete = 0").expect("off");

    let inside = erase_rows::with_secure_delete(&mut conn, |conn| Ok(setting(conn))).expect("ok");
    assert_eq!(inside, 1, "on for the work");
    assert_eq!(setting(&conn), 0, "back off afterwards");

    let failed: comemory::prelude::Result<()> = erase_rows::with_secure_delete(&mut conn, |_| {
        Err(comemory::prelude::Error::Other("boom".to_string()))
    });
    assert!(failed.is_err(), "the work's error is returned");
    assert_eq!(setting(&conn), 0, "and the setting is restored anyway");
}

#[test]
fn truncate_wal_reports_a_reader_that_outlasts_the_wait() {
    let home = TempDir::new().expect("tempdir");
    let (paths, cfg, mut conn) = open(home.path());
    save(
        &paths,
        &cfg,
        &mut conn,
        "a write that leaves frames in the WAL",
        "Frames",
    );
    let reader = Connection::open(paths.db_path()).expect("reader");
    reader.execute_batch("BEGIN").expect("begin read");
    let _: i64 = reader
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .expect("read inside the transaction");
    save(
        &paths,
        &cfg,
        &mut conn,
        "a later write the reader cannot see",
        "Later",
    );
    conn.execute_batch("PRAGMA busy_timeout = 100")
        .expect("short timeout");

    let held = erase_rows::truncate_wal(&conn, std::time::Duration::from_millis(300))
        .expect("checkpoint attempt");
    assert!(!held, "a reader holding an old snapshot keeps the WAL");

    reader.execute_batch("COMMIT").expect("end read");
    assert!(
        erase_rows::truncate_wal(&conn, std::time::Duration::from_secs(5)).expect("checkpoint"),
        "once the reader is gone the WAL truncates"
    );
}

/// Index one guide as a shared document through the real writer, the way
/// the writer's own tests do, and return its shared id.
fn shared_document(conn: &mut Connection, root: &Path) -> String {
    use comemory::domains::documents::document::DocumentFormat;
    use comemory::domains::documents::document::writer;
    use comemory::domains::documents::source::classify::Classification;
    use comemory::domains::documents::source::discover::Candidate;
    comemory::store::sources::upsert(
        conn,
        comemory::store::sources::SourceRootUpsert {
            id: "source-under-test",
            canonical_path: root.to_str().expect("utf8"),
            kind: "dir",
            repo: Some("Falconiere/comemory"),
            created_at: NOW,
            updated_at: NOW,
        },
    )
    .expect("source root");
    comemory::store::repository_approval::replace_all(
        conn,
        &[(
            "Falconiere/comemory".to_string(),
            "Falconiere/comemory".to_string(),
        )],
        NOW,
    )
    .expect("approve");
    conn.execute(
        "INSERT INTO repo_marker (repo, root_path) VALUES ('Falconiere/comemory', ?1)",
        [root.to_str().expect("utf8")],
    )
    .expect("root");
    let path = root.join("guide.md");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/common/fixtures/docs/guide.md"),
        &path,
    )
    .expect("copy guide");
    let candidate = Candidate {
        relative_path: "guide.md".into(),
        absolute_path: path,
        classification: Classification::Document(DocumentFormat::Markdown),
    };
    writer::update_file(
        conn,
        "source-under-test",
        Some("Falconiere/comemory"),
        &candidate,
        root,
        16 * 1024 * 1024,
    )
    .expect("index");
    comemory::domains::documents::share::shared_id("Falconiere/comemory", "guide.md")
}

#[test]
fn erase_local_document_removes_its_rows_and_never_its_file() {
    let home = TempDir::new().expect("tempdir");
    let root = fs::canonicalize(home.path()).expect("canonical");
    let (_paths, _cfg, mut conn) = open(&root.join("data"));
    let shared_id = shared_document(&mut conn, &root);

    let local = erase_rows::local_documents(&conn, &shared_id).expect("lookup");
    assert_eq!(local.len(), 1, "the shared document is found by its name");
    let LocalDocument {
        document_id,
        source_file_id,
    } = local[0].clone();

    let tx = conn.transaction().expect("begin");
    erase_rows::erase_local_document(&tx, &local[0]).expect("erase");
    tx.commit().expect("commit");

    for (table, sql, key) in [
        (
            "documents",
            "SELECT COUNT(*) FROM documents WHERE id = ?1",
            &document_id,
        ),
        (
            "document_chunks",
            "SELECT COUNT(*) FROM document_chunks WHERE document_id = ?1",
            &document_id,
        ),
        (
            "document_fts",
            "SELECT COUNT(*) FROM document_fts WHERE document_id = ?1",
            &document_id,
        ),
        (
            "document_share",
            "SELECT COUNT(*) FROM document_share WHERE document_id = ?1",
            &document_id,
        ),
        (
            "source_files",
            "SELECT COUNT(*) FROM source_files WHERE id = ?1",
            &source_file_id,
        ),
    ] {
        assert_eq!(
            count(&conn, sql, key),
            0,
            "{table} still holds the document"
        );
    }
    assert!(
        root.join("guide.md").exists(),
        "the source file is untouched"
    );
    assert!(
        erase_rows::local_documents(&conn, &shared_id)
            .expect("lookup")
            .is_empty()
    );
}

#[test]
fn erase_pulled_document_removes_the_copy_under_every_repository_and_nothing_else() {
    let home = TempDir::new().expect("tempdir");
    let (_paths, _cfg, conn) = open(home.path());
    let erased = "0f3c9a1e5b7d4c2a8e6f1b3d5a7c9e0f";
    let kept = "1e2d3c4b5a69788796a5b4c3d2e1f001";
    for (repo, shared_id) in [
        ("Falconiere/comemory", erased),
        ("Falconiere/other", erased),
        ("Falconiere/comemory", kept),
    ] {
        remote_document::replace_revision(
            &conn,
            &Revision {
                repo: repo.to_string(),
                shared_id: shared_id.to_string(),
                path: "docs/guides/erase.md".to_string(),
                title: "Erase".to_string(),
                format: "markdown".to_string(),
                revision_hash: "rev".to_string(),
                chunk_count: 1,
            },
            &[Chunk {
                ordinal: 0,
                heading_path: String::new(),
                char_range: (0, 10),
                line_range: (1, 1),
                simhash: 0,
                text: format!("pulled passage of {shared_id}"),
            }],
            &[],
            NOW,
        )
        .expect("pulled copy");
    }

    assert!(erase_rows::erase_pulled_document(&conn, erased).expect("erase"));

    for table in [
        "remote_document",
        "remote_document_chunk",
        "remote_document_link",
        "remote_document_fts",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE shared_id = ?1");
        assert_eq!(count(&conn, &sql, erased), 0, "{table} keeps a copy");
    }
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM remote_document_chunk WHERE shared_id = ?1",
            kept
        ),
        1,
        "another document's copy is untouched"
    );
    assert!(
        !erase_rows::erase_pulled_document(&conn, erased).expect("again"),
        "nothing left to erase"
    );
}

#[test]
fn every_store_connection_zeroes_what_it_frees() {
    // An erase can only scrub the rows it still finds. The FTS row a soft
    // delete removed, or the old copy of a row an update moved, must already
    // have been zeroed by whichever connection freed it —
    // `maintenance::erase`'s trashed-memory test is the end-to-end proof.
    let home = TempDir::new().expect("tempdir");
    let (_paths, _cfg, conn) = open(home.path());
    let setting: i64 = conn
        .query_row("PRAGMA secure_delete", [], |r| r.get(0))
        .expect("pragma");
    assert_eq!(setting, 1);
}

#[test]
fn a_complete_staged_set_hashing_to_an_erased_digest_is_deleted_and_nothing_else() {
    use comemory::store::replica_staging;
    let home = TempDir::new().expect("tempdir");
    let (_paths, _cfg, conn) = open(home.path());
    // A two-part upload whose assembled bytes are the erased payload — split
    // mid-document, and not canonical, so the digest has to be taken over the
    // parsed value exactly as activation takes it.
    let payload = serde_json::json!({"body": "erased text", "id": "a1b2c3d4"});
    let (_, erased) =
        comemory::utilities::canonical_json::bytes_and_digest(&payload).expect("digest");
    let spaced = r#"{ "id": "a1b2c3d4", "body": "erased text" }"#;
    let (head, tail) = spaced.split_at(12);
    for (index, part) in [head, tail].into_iter().enumerate() {
        replica_staging::put_part(
            &conn,
            "set-erased",
            index as i64,
            2,
            part,
            "2026-09-25T10:00:00Z",
        )
        .expect("stage");
    }
    replica_staging::put_part(
        &conn,
        "set-other",
        0,
        1,
        r#"{"body":"kept"}"#,
        "2026-09-25T10:00:00Z",
    )
    .expect("stage other");
    // Incomplete: one of two parts. It cannot be attributed, so it stays for
    // the sweep even though its first part is the erased set's first part.
    replica_staging::put_part(&conn, "set-partial", 0, 2, head, "2026-09-25T10:00:00Z")
        .expect("stage partial");

    let cleared = erase_rows::erase_staged_of(&conn, &[erased]).expect("clear");

    assert_eq!(cleared, 2, "both parts of the complete erased set");
    let left: Vec<String> = conn
        .prepare("SELECT DISTINCT staging_id FROM replica_staged_part ORDER BY 1")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");
    assert_eq!(
        left,
        vec!["set-other".to_string(), "set-partial".to_string()]
    );
    assert_eq!(
        erase_rows::erase_staged_of(&conn, &[]).expect("no digests"),
        0
    );
}

#[test]
fn erase_memory_blanks_its_captured_passages_and_their_locators() {
    use comemory::store::candidate_observations::{NewCandidate, NewObservation, insert};
    let home = TempDir::new().expect("tempdir");
    let (_paths, _cfg, conn) = open(home.path());
    let candidate = |position: i64, reference: &'static str, text: &'static str| NewCandidate {
        pool_position: position,
        domain: "memory",
        candidate_ref: reference,
        content_version: "hash",
        unresolved: false,
        returned_position: Some(position),
        retrieval_score: 0.5,
        rank_in_domain: position,
        tier: Some(1),
        text,
        text_sha256: "d0",
        text_full_bytes: 20,
        text_truncated: false,
        locator_json: r#"{"title":"The erased first line","path":"/m/aaaa0001-the-erased.md"}"#,
    };
    insert(
        &conn,
        &NewObservation {
            observation_id: "o-20260925-00000001",
            observation_version: 1,
            query_id: Some("q-20260925-aaaaaaaa"),
            query: "the words a user typed",
            source: "find",
            filters_json: "{}",
            retrieval_json: "{}",
            knobs_hash: "k0",
            corpus_digest: "c0",
            decay_frozen: false,
            pool_size: 2,
            page_limit: 2,
            page_offset: 0,
            truncated: false,
            at: NOW,
        },
        &[
            candidate(1, "memory:aaaa0001:hash", "the erased passage"),
            candidate(2, "memory:bbbb0002:hash", "a kept passage"),
        ],
    )
    .expect("capture");

    let tx = conn.unchecked_transaction().expect("begin");
    erase_rows::erase_memory(&tx, "aaaa0001", NOW).expect("erase");
    tx.commit().expect("commit");

    let row = |position: i64| -> (String, String, bool) {
        conn.query_row(
            "SELECT text, locator_json, unresolved FROM candidate_observations \
              WHERE pool_position = ?1",
            [position],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("candidate row")
    };
    let (text, locator, unresolved) = row(1);
    assert_eq!(text, "", "the passage is blanked, as a purge blanks it");
    assert!(unresolved);
    let locator: serde_json::Value = serde_json::from_str(&locator).expect("json");
    assert_eq!(locator["title"], serde_json::json!(""), "{locator}");
    assert!(locator["path"].is_null(), "{locator}");
    let (text, locator, unresolved) = row(2);
    assert_eq!(text, "a kept passage");
    assert!(!unresolved);
    assert!(
        locator.contains("The erased first line"),
        "another memory's locator is kept"
    );
    let query: String = conn
        .query_row("SELECT query FROM candidate_query_observations", [], |r| {
            r.get(0)
        })
        .expect("header");
    assert_eq!(
        query, "the words a user typed",
        "the query a user typed is kept"
    );
}
