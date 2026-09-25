#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/maintenance/rebuild.rs`. `maintenance::rebuild::run` is called
//! directly (no CLI process) against real temp data-dirs seeded by real
//! `comemory save` runs: the markdown replay, the atomic swap's sidecar
//! cleanup, and the leave-the-live-DB-alone error path. The preservation
//! copy is covered in `tests/api__rebuild__copy.rs`; the CLI's own
//! byte-compat coverage stays in `tests/cli__rebuild.rs` /
//! `tests/cli__rebuild_2.rs`, and the HTTP job route in
//! `tests/serve__routes__maint__admin.rs`.

use crate::test_common::cli_rebuild_support as support;

use comemory::config::{Config, Paths};
use comemory::domains::maintenance;
use comemory::utilities::context::Ctx;
use rusqlite::Connection;
use tempfile::{TempDir, tempdir};

use support::{count, open_db, open_db_with_vec, run_save};

/// Call `maintenance::rebuild::run` over `home`'s data dir with a `Ctx::lazy` — the
/// same shape the HTTP job uses, and the one that proves `rebuild` never
/// needs a caller-owned connection.
fn run_rebuild_api(home: &TempDir) -> comemory::prelude::Result<()> {
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    maintenance::rebuild::run(&mut ctx, maintenance::rebuild::Request {})
}

fn assert_edge(conn: &Connection, rel: &str, dst_kind: &str, dst_id: &str) {
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM edges WHERE src_kind = 'memory' AND rel = ?1 \
               AND dst_kind = ?2 AND dst_id = ?3",
            rusqlite::params![rel, dst_kind, dst_id],
            |r| r.get(0),
        )
        .expect("count edges");
    assert_eq!(n, 1, "expected edge {rel} -> {dst_kind}:{dst_id}");
}

#[test]
fn run_reconstructs_memories_from_markdown() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "body one"]);
    run_save(&home, &["--kind", "note", "body two"]);
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");

    run_rebuild_api(&home).expect("rebuild");

    let conn = open_db(&home);
    assert_eq!(count(&conn, "SELECT count(*) FROM memories"), 2);
}

#[test]
fn run_restores_tags_fts_and_edges() {
    let home = tempdir().expect("tempdir");
    run_save(
        &home,
        &[
            "--kind",
            "decision",
            "--repo",
            "qwick",
            "--author",
            "alice",
            "--tags",
            "db,postgres",
            "see `qwick:src/lib.rs` for the rationale",
        ],
    );
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");

    run_rebuild_api(&home).expect("rebuild");

    let conn = open_db_with_vec(&home);
    assert_eq!(count(&conn, "SELECT count(*) FROM memories"), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM memory_tags"), 2);
    let fts_hits = count(
        &conn,
        "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'rationale'",
    );
    assert!(fts_hits >= 1, "expected FTS to find 'rationale'");
    assert_edge(&conn, "in_repo", "repo", "qwick");
    assert_edge(&conn, "authored_by", "author", "alice");
    assert_edge(&conn, "tagged", "tag", "db");
    assert_edge(&conn, "tagged", "tag", "postgres");
    assert_edge(&conn, "references_file", "file", "qwick:src/lib.rs");
}

#[test]
fn run_skips_hidden_staging_files() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "kept body"]);
    // A stale staging file in `memories/` must be skipped, not parsed as
    // YAML frontmatter.
    let stale = home.path().join("memories").join(".abc12345.tmp");
    std::fs::write(&stale, "garbage not yaml").expect("write stale");
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");

    run_rebuild_api(&home).expect("rebuild");

    let conn = open_db(&home);
    assert_eq!(count(&conn, "SELECT count(*) FROM memories"), 1);
}

#[test]
fn run_cleans_up_the_tmp_db_and_its_wal_shm_sidecars() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "sidecar cleanup body"]);

    run_rebuild_api(&home).expect("rebuild");

    let live = home.path().join("comemory.db");
    let tmp = home.path().join("comemory.db.rebuild.tmp");
    for path in [&live, &tmp] {
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.clone().into_os_string();
            sidecar.push(suffix);
            let sidecar = std::path::PathBuf::from(sidecar);
            assert!(
                !sidecar.exists(),
                "rebuild must not leave a sidecar at {}",
                sidecar.display()
            );
        }
    }
    assert!(!tmp.exists(), "rebuild must not leave the tmp DB behind");
}

#[test]
fn run_leaves_the_live_db_intact_when_the_tmp_path_is_blocked() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "original body"]);
    let original = {
        let conn = open_db(&home);
        count(&conn, "SELECT count(*) FROM memories")
    };

    // SQLite cannot open a directory as a database, so `connection::open`
    // on the tmp path fails well before the rename that would clobber the
    // live DB.
    let tmp_path = home.path().join("comemory.db.rebuild.tmp");
    std::fs::create_dir_all(&tmp_path).expect("create dir at tmp path");

    let err = run_rebuild_api(&home).expect_err("blocked tmp path must error");
    // `connection::open(tmp_path)` calls `rusqlite::Connection::open` on a
    // path that is a directory — SQLite refuses with `SQLITE_CANTOPEN`,
    // rendered by rusqlite's `Display` as "unable to open database file:
    // <path>" and wrapped verbatim into `Error::Sqlite`'s "sqlite: {0}".
    let msg = err.to_string();
    assert!(
        msg.contains("unable to open database file"),
        "expected the real SQLite cannot-open failure, got: {msg}"
    );
    assert!(
        msg.contains(tmp_path.to_string_lossy().as_ref()),
        "expected the blocked tmp path in the error, got: {msg}"
    );

    let conn = open_db(&home);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM memories"),
        original,
        "the original DB must survive a failed rebuild"
    );
}

#[test]
fn run_on_a_fresh_data_dir_with_no_memories_builds_an_empty_mirror() {
    let home = tempdir().expect("tempdir");

    run_rebuild_api(&home).expect("rebuild on a fresh data dir");

    let conn = open_db(&home);
    assert_eq!(count(&conn, "SELECT count(*) FROM memories"), 0);
}

/// [`maintenance::rebuild::snapshot_before_swap`]'s whole reason to exist: the live
/// `comemory.db` — as it stood BEFORE this rebuild rewrote it — must be
/// recoverable from `comemory.db.pre-rebuild.bak` immediately after.
///
/// The memory's markdown file is removed BEFORE the rebuild runs, so the
/// rebuild's markdown replay can no longer reconstruct that row and the
/// pre-/post-rebuild database states are genuinely different: the still-live
/// db `snapshot_before_swap` captures still has the row (deleting the `.md`
/// file never touched `comemory.db`), while the freshly rebuilt db does not
/// (there is no markdown left to replay it from). Without this the row would
/// land in BOTH databases via the replay, and the assertions below would
/// pass even if the snapshot ran AFTER the swap instead of before it —
/// mutation-tested by moving the `snapshot_before_swap` call to after
/// `swap_into_place` in `src/domains/maintenance/rebuild.rs::run`, which turns the
/// `n_in_live` assertion red.
#[test]
fn run_snapshots_the_pre_rebuild_db_before_the_swap() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "pre-rebuild snapshot body"]);

    let before_id: String = {
        let conn = open_db(&home);
        conn.query_row("SELECT id FROM memories LIMIT 1", [], |r| r.get(0))
            .expect("pre-rebuild memory id")
    };

    let md_file = only_markdown_file(&home);
    std::fs::remove_file(&md_file).expect("remove the memory's markdown file");

    run_rebuild_api(&home).expect("rebuild");

    let bak = home.path().join("comemory.db.pre-rebuild.bak");
    assert!(
        bak.exists(),
        "rebuild must snapshot the live db to comemory.db.pre-rebuild.bak before the swap"
    );

    let snap = Connection::open(&bak).expect("open pre-rebuild snapshot");
    let n_in_snapshot: i64 = snap
        .query_row(
            "SELECT count(*) FROM memories WHERE id = ?1",
            [&before_id],
            |r| r.get(0),
        )
        .expect("count memories in snapshot");
    assert_eq!(
        n_in_snapshot, 1,
        "pre-rebuild snapshot must contain the row that existed before the rebuild, even \
         though its markdown file was removed before the rebuild ran"
    );

    let live = open_db(&home);
    let n_in_live: i64 = live
        .query_row(
            "SELECT count(*) FROM memories WHERE id = ?1",
            [&before_id],
            |r| r.get(0),
        )
        .expect("count memories in the post-rebuild live db");
    assert_eq!(
        n_in_live, 0,
        "the post-rebuild live db must NOT contain a row whose markdown file was removed \
         before the rebuild ran — this is what proves the snapshot captured the state \
         BEFORE the swap, not the state after it"
    );
}

/// The single `*.md` memory file under `home`'s `memories/` directory.
///
/// # Panics
///
/// Panics if the directory cannot be read or does not hold exactly one
/// `.md` file.
fn only_markdown_file(home: &TempDir) -> std::path::PathBuf {
    let dir = home.path().join("memories");
    let mut matches: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .expect("read memories dir")
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one markdown file under memories/, found {matches:?}"
    );
    matches.remove(0)
}

/// [`maintenance::rebuild::snapshot_before_swap_inner`] snapshots to a sibling
/// staging path and renames it over `.bak` only on success, so a failed
/// attempt at a NEW snapshot must never destroy a prior GOOD one. Blocks the
/// staging path (`comemory.db.pre-rebuild.bak.tmp`), not `.bak` itself, so
/// the failure happens before anything ever touches the existing backup.
#[test]
fn run_preserves_the_prior_pre_rebuild_backup_when_the_new_snapshot_fails() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "first body"]);

    // First rebuild succeeds and snapshots the then-live db (one memory).
    run_rebuild_api(&home).expect("first rebuild");
    let bak = home.path().join("comemory.db.pre-rebuild.bak");
    assert!(
        bak.exists(),
        "first rebuild must write the pre-rebuild backup"
    );
    let before_count: i64 = {
        let snap = Connection::open(&bak).expect("open first backup");
        count(&snap, "SELECT count(*) FROM memories")
    };
    assert_eq!(before_count, 1);

    // A second save grows the live db, so a fresh snapshot would differ
    // from the first one if the second rebuild's snapshot succeeded.
    run_save(&home, &["--kind", "note", "second body"]);

    let staging = home.path().join("comemory.db.pre-rebuild.bak.tmp");
    std::fs::create_dir_all(&staging).expect("block the staging path with a directory");

    let err =
        run_rebuild_api(&home).expect_err("a blocked staging path must fail the second rebuild");
    let msg = err.to_string();
    assert!(
        msg.contains(&bak.to_string_lossy().to_string()),
        "the wrapped error must name the pre-rebuild backup destination, got: {msg}"
    );

    let after_count: i64 = {
        let snap = Connection::open(&bak).expect("open backup after the failed rebuild");
        count(&snap, "SELECT count(*) FROM memories")
    };
    assert_eq!(
        after_count, before_count,
        "a failed new snapshot must not disturb the prior good .bak"
    );
}

/// A blocked `.bak` destination must abort the whole rebuild — not silently
/// skip the snapshot and swap in the new DB anyway — and leave the live DB
/// fully usable. Pre-creating the path as a DIRECTORY blocks it
/// deterministically: `fs::remove_file` refuses to unlink a directory
/// regardless of uid (this sandbox runs as uid 0, so a chmod-based block
/// would be inert — directory mode bits are ignored for root).
#[test]
fn run_aborts_when_the_rebuild_backup_path_is_blocked_and_leaves_the_live_db_intact() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "blocked backup body"]);
    let original = {
        let conn = open_db(&home);
        count(&conn, "SELECT count(*) FROM memories")
    };

    let bak = home.path().join("comemory.db.pre-rebuild.bak");
    std::fs::create_dir_all(&bak).expect("block the bak path with a directory");

    let err = run_rebuild_api(&home).expect_err("a blocked backup path must abort the rebuild");
    let msg = err.to_string();
    assert!(
        msg.contains(&bak.to_string_lossy().to_string()),
        "the wrapped error must name the blocked destination, got: {msg}"
    );

    let tmp = home.path().join("comemory.db.rebuild.tmp");
    assert!(
        !tmp.exists(),
        "an aborted rebuild must not leave the tmp db behind"
    );

    let conn = open_db(&home);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM memories"),
        original,
        "the live db must remain fully usable after an aborted rebuild"
    );
}

// ---------------------------------------------------------------------------
// #251: a rebuild re-derives local state from markdown. It is materialization,
// not a mutation, so it must journal nothing — and it must not lose the
// positions it already handed to peers.
// ---------------------------------------------------------------------------

/// Feed positions as `"<op>:<entity_key>"`, oldest first.
fn feed_ops(conn: &Connection) -> Vec<String> {
    comemory::store::replica_read::page(conn, 0, 50, None)
        .expect("page")
        .into_iter()
        .map(|row| format!("{}:{}", row.op.as_str(), row.entity_key))
        .collect()
}

#[test]
fn a_rebuild_replays_every_memory_and_adds_no_feed_position() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "rebuild subject one"]);
    run_save(&home, &["--kind", "note", "rebuild subject two"]);
    let before = {
        let conn = open_db(&home);
        feed_ops(&conn)
    };
    let outbox_before = {
        let conn = open_db(&home);
        comemory::store::replica_outbox::count(&conn, "pending").expect("count")
    };
    assert_eq!(before.len(), 2, "two saves, two positions");

    run_rebuild_api(&home).expect("rebuild");

    let conn = open_db(&home);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM memories"),
        2,
        "the replay demonstrably rebuilt the mirror"
    );
    assert_eq!(
        feed_ops(&conn),
        before,
        "a rebuild re-derives local state; it neither invents nor drops a \
         position a peer already holds a receipt for"
    );
    assert_eq!(
        comemory::store::replica_outbox::count(&conn, "pending").expect("count"),
        outbox_before,
        "and it owes no new upload"
    );
    assert_eq!(
        count(&conn, "SELECT count(*) FROM memory_write_intent"),
        0,
        "a rebuild IS the reconciliation an outstanding intent would ask for, \
         so it leaves none behind"
    );
}

// ---------------------------------------------------------------------------
// AC-11: a rebuild carries what no local file could re-derive. Listing a table
// in `COPIED_TABLES` satisfies the coverage test and copies nothing on its
// own, so these assert the rows themselves.
//
// The local half is registered through the real `index::run`, which writes
// `sources.toml`: `source_roots` is RECONSTRUCTED from that file before the
// copy runs, and `source_files.source_id` references it, so a fixture that
// seeded those rows directly would make the copy fail on the missing parent
// rather than prove anything.
// ---------------------------------------------------------------------------

const SHARE_REPO: &str = "Falconiere/comemory";
const SHARE_AT: &str = "2026-09-23T10:00:00Z";
const SHARED_PATH: &str = "docs/guides/cloud-sync.md";

/// Index this repository's own `docs/guides` into `home` under `SHARE_REPO`,
/// approved and rooted, so the run really shares what it indexes.
fn index_real_guides(home: &TempDir) -> TempDir {
    let workspace = tempdir().expect("workspace");
    let docs = workspace.path().join("docs").join("guides");
    std::fs::create_dir_all(&docs).expect("mkdir");
    for name in ["http-api.md", "replication-e2e.md"] {
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("docs/guides")
                .join(name),
            docs.join(name),
        )
        .expect("copy real guide");
    }
    {
        let conn = open_db_with_vec(home);
        comemory::store::repository_approval::replace_all(
            &conn,
            &[(SHARE_REPO.to_string(), SHARE_REPO.to_string())],
            SHARE_AT,
        )
        .expect("approve");
        let root = std::fs::canonicalize(workspace.path()).expect("canonicalize");
        conn.execute(
            "INSERT INTO repo_marker (repo, root_path) VALUES (?1, ?2)",
            rusqlite::params![SHARE_REPO, root.to_str().expect("utf8")],
        )
        .expect("record the root");
    }
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut conn = open_db_with_vec(home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let output = comemory::domains::documents::index::run(
        &mut ctx,
        comemory::domains::documents::index::Request {
            path: vec![docs.to_str().expect("utf8 path").to_string()],
            repo: Some(SHARE_REPO.to_string()),
            strict: false,
        },
    )
    .expect("index the real guides");
    assert_eq!(output.sources[0].indexed, 2, "{output:?}");
    workspace
}

/// Put a real pulled revision of a document this machine has NO file for.
fn hold_pulled_revision(home: &TempDir) -> String {
    use comemory::domains::documents::document::DocumentFormat;
    use comemory::domains::documents::document::extract::extract;
    use comemory::store::remote_document::{self, Chunk, Link, Revision};

    let conn = open_db_with_vec(home);
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(SHARED_PATH);
    let bytes = std::fs::read(&file).expect("read the real guide");
    let doc = extract(DocumentFormat::Markdown, &bytes, "doc").expect("real extraction");
    let shared_id = comemory::domains::documents::share::shared_id(SHARE_REPO, SHARED_PATH);
    let revision = Revision {
        repo: SHARE_REPO.to_string(),
        shared_id: shared_id.clone(),
        path: SHARED_PATH.to_string(),
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
    remote_document::replace_revision(
        &conn,
        &revision,
        &chunks,
        &[Link {
            ordinal: 0,
            target: "docs/guides/http-api.md".to_string(),
        }],
        SHARE_AT,
    )
    .expect("hold a pulled revision");
    shared_id
}

/// The six counts a rebuild must preserve.
fn shared_counts(home: &TempDir) -> [i64; 6] {
    let conn = open_db_with_vec(home);
    [
        count(&conn, "SELECT COUNT(*) FROM remote_document"),
        count(&conn, "SELECT COUNT(*) FROM remote_document_chunk"),
        count(&conn, "SELECT COUNT(*) FROM remote_document_link"),
        count(&conn, "SELECT COUNT(*) FROM remote_document_fts"),
        count(&conn, "SELECT COUNT(*) FROM document_share"),
        count(&conn, "SELECT COUNT(*) FROM repository_approval"),
    ]
}

#[test]
fn a_rebuild_keeps_every_pulled_revision_and_share_mapping() {
    let home = tempdir().expect("tempdir");
    run_save(
        &home,
        &[
            "--kind",
            "note",
            "a memory so the replay has something to do",
        ],
    );
    let _workspace = index_real_guides(&home);
    let shared_id = hold_pulled_revision(&home);
    let before = shared_counts(&home);
    assert!(
        before.iter().all(|n| *n > 0),
        "the fixture really seeded the state a rebuild must carry: {before:?}"
    );

    run_rebuild_api(&home).expect("rebuild");

    assert_eq!(
        shared_counts(&home),
        before,
        "nothing on this disk could re-derive a peer's revision, or what a \
         local document is called upstream, so a rebuild carries every row"
    );
    let conn = open_db_with_vec(&home);
    let held = comemory::store::remote_document::revision(&conn, SHARE_REPO, &shared_id)
        .expect("read")
        .expect("the revision survived");
    assert_eq!(held.path, SHARED_PATH);
    assert_eq!(held.revision_hash, "a".repeat(64));
    assert_eq!(
        comemory::store::repository_approval::canonical_for(&conn, SHARE_REPO)
            .expect("read")
            .as_deref(),
        Some(SHARE_REPO),
        "the approval map too, or every document would be withheld until the \
         next policy load"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM source_roots"),
        1,
        "and `source_roots` is still reconstructed from sources.toml, not copied"
    );
}

#[test]
fn every_share_mapping_a_rebuild_keeps_still_names_a_document() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "a memory"]);
    let _workspace = index_real_guides(&home);
    let before = count(
        &open_db_with_vec(&home),
        "SELECT COUNT(*) FROM document_share",
    );
    assert!(before > 0, "the index run shared what it indexed");

    run_rebuild_api(&home).expect("rebuild");

    let conn = open_db_with_vec(&home);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM document_share"),
        before,
        "the mappings survive with the documents they name"
    );
    // `document_share.document_id` is a foreign key, so a mapping naming a
    // document the copy did not carry would have aborted the rebuild rather
    // than reaching this assertion. Checking the join anyway states the
    // invariant the copy's narrowing exists to keep.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM document_share s \
              WHERE NOT EXISTS (SELECT 1 FROM documents d WHERE d.id = s.document_id)",
        ),
        0,
        "no mapping is left naming a document that does not exist"
    );
}
