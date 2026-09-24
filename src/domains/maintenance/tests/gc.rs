#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/maintenance/gc.rs`. Calls `maintenance::gc::run` directly against a
//! `Ctx::lazy` opened on a temp data-dir, proving the trash sweep,
//! telemetry retention purge, and — critically — that a fresh data dir with
//! no prior `comemory.db` is never touched by `run` (`cli::gc::run` is
//! byte-compat tested against CLI stdout in `tests/cli__gc.rs`; the HTTP
//! route lives in `tests/serve__routes__maint__prune.rs`). The second half
//! drives real `save` → `delete` → aged-mtime → `gc` cycles over a
//! `Ctx::borrowed` and proves a reaped file's mirror rows go with it, a
//! zombie row (file already gone, `deleted_at` past the window) is purged,
//! and a live memory or a fresh trash entry is never touched.

use comemory::config::{Config, Paths};
use comemory::domains::maintenance;
use comemory::utilities::context::Ctx;
use time::{Duration, OffsetDateTime};

fn db_path(home: &tempfile::TempDir) -> std::path::PathBuf {
    home.path().join("comemory.db")
}

fn seed_telemetry(home: &tempfile::TempDir) {
    std::fs::create_dir_all(home.path()).expect("create data dir");
    let conn = comemory::store::connection::open(db_path(home)).expect("open + migrate db");
    let now = OffsetDateTime::now_utc();
    let old =
        comemory::store::memory_row::iso_format(now - Duration::days(100)).expect("old stamp");
    let fresh =
        comemory::store::memory_row::iso_format(now - Duration::days(1)).expect("fresh stamp");
    for (qid, at) in [("q-old", &old), ("q-new", &fresh)] {
        conn.execute(
            "INSERT INTO retrieval_log(query_id, query, returned_ids, at) \
             VALUES (?1, 'some query', '[]', ?2)",
            rusqlite::params![qid, at],
        )
        .expect("insert retrieval_log row");
    }
}

#[test]
fn run_on_a_fresh_data_dir_never_creates_the_db() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    assert!(!db_path(&home).exists(), "db must not exist yet");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = maintenance::gc::run(&mut ctx, maintenance::gc::Request {}).expect("gc run");

    assert_eq!(resp.removed, 0);
    assert_eq!(resp.log_rows, 0);
    assert_eq!(resp.event_rows, 0);
    assert_eq!(resp.bytes_freed, 0);
    assert!(
        !db_path(&home).exists(),
        "gc on a fresh dir must not create comemory.db"
    );
}

#[test]
fn run_sweeps_old_telemetry_when_the_db_already_exists() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    seed_telemetry(&home);

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = maintenance::gc::run(&mut ctx, maintenance::gc::Request {}).expect("gc run");

    assert_eq!(resp.removed, 0);
    assert_eq!(resp.log_rows, 1, "one old retrieval_log row swept");
    assert_eq!(resp.event_rows, 0);
    assert_eq!(resp.bytes_freed, 0, "no trash swept in this run");

    let conn = rusqlite::Connection::open_with_flags(
        db_path(&home),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("reopen db read-only");
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM retrieval_log", [], |r| r.get(0))
        .expect("count retrieval_log");
    assert_eq!(remaining, 1, "fresh row must survive");

    let (removed, log_rows, bytes_freed): (i64, i64, i64) = conn
        .query_row(
            "SELECT removed, log_rows, bytes_freed FROM gc_runs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("gc_runs row written when the db already exists");
    assert_eq!(removed, 0);
    assert_eq!(log_rows, 1);
    assert_eq!(bytes_freed, 0);
}

#[test]
fn run_sweeps_trash_entries_older_than_thirty_days() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let trash = paths.trash_dir();
    std::fs::create_dir_all(&trash).expect("create trash dir");
    let old = trash.join("11111111-old.md");
    let fresh = trash.join("22222222-fresh.md");
    std::fs::write(&old, "old").expect("write old trash entry");
    std::fs::write(&fresh, "fresh").expect("write fresh trash entry");
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&old)
        .expect("reopen old trash entry");
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_hours(31 * 24))
        .expect("backdate mtime");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = maintenance::gc::run(&mut ctx, maintenance::gc::Request {}).expect("gc run");

    assert_eq!(resp.removed, 1);
    assert_eq!(resp.bytes_freed, 3, "the removed \"old\" file is 3 bytes");
    assert!(!old.exists(), "old trash entry must be deleted");
    assert!(fresh.exists(), "fresh trash entry must be kept");
    assert!(
        !db_path(&home).exists(),
        "sweeping trash alone (no seeded db) must not create comemory.db"
    );
}

/// A migrated data dir plus a borrowed connection, the shape the purge
/// tests drive `maintenance::gc::run` through (a `Ctx::borrowed`, like the CLI).
fn open_store(home: &tempfile::TempDir) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let conn = comemory::store::connection::open(paths.db_path()).expect("open + migrate db");
    (paths, Config::defaults(), conn)
}

/// Save one note through the real `domains::memories::save::run`, returning its id.
fn save_note(paths: &Paths, cfg: &Config, conn: &mut rusqlite::Connection, body: &str) -> String {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    crate::domains::memories::save::run(
        &mut ctx,
        crate::domains::memories::save::Request {
            body: body.to_string(),
            title: None,
            kind: comemory::domains::memories::Kind::Note,
            repo: "demo".to_string(),
            tags: vec!["gc".to_string()],
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

/// Soft-delete `id` through the real `domains::memories::delete::run` and return the
/// `.trash/` path the file landed at.
fn soft_delete(
    paths: &Paths,
    cfg: &Config,
    conn: &mut rusqlite::Connection,
    id: &str,
) -> std::path::PathBuf {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    crate::domains::memories::delete::run(&mut ctx, id).expect("soft delete");
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    let page = crate::domains::memories::trash::run(
        &mut ctx,
        crate::domains::memories::trash::Request {
            limit: 0,
            offset: 0,
        },
    )
    .expect("trash listing");
    let row = page
        .items
        .iter()
        .find(|r| r.id == id)
        .expect("soft-deleted memory is listed in the trash");
    std::path::PathBuf::from(row.path.clone().expect("trashed file is on disk"))
}

/// Age `path`'s mtime past the 30-day trash window with a real mtime
/// rewrite, never a faked clock.
fn backdate(path: &std::path::Path) {
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("reopen trashed file");
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_hours(31 * 24))
        .expect("backdate mtime");
}

fn gc(paths: &Paths, cfg: &Config, conn: &mut rusqlite::Connection) -> maintenance::gc::Response {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    maintenance::gc::run(&mut ctx, maintenance::gc::Request {}).expect("gc run")
}

fn trash_ids(paths: &Paths, cfg: &Config, conn: &mut rusqlite::Connection) -> Vec<String> {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    crate::domains::memories::trash::run(
        &mut ctx,
        crate::domains::memories::trash::Request {
            limit: 0,
            offset: 0,
        },
    )
    .expect("trash listing")
    .items
    .into_iter()
    .map(|r| r.id)
    .collect()
}

/// Write a `memory_vec` row for `id` through the real vector writer. The
/// soft delete drops any vector the save wrote, so a purge assertion is
/// only meaningful against a row that exists at purge time — this is the
/// caller re-embedding a trashed id, which `reembed` can genuinely do.
fn seed_vector(conn: &rusqlite::Connection, id: &str) {
    let dim = comemory::store::vector::dim_memory(conn).expect("memory dim");
    comemory::store::vector::insert_memory(conn, id, &vec![0.25; dim]).expect("insert vec row");
}

fn count_by_id(conn: &rusqlite::Connection, sql: &str, id: &str) -> i64 {
    conn.query_row(sql, [id], |r| r.get(0)).expect("count")
}

#[test]
fn run_purges_the_mirror_rows_behind_a_reaped_trash_file() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_store(&home);
    let id = save_note(&paths, &cfg, &mut conn, "reaped together with its rows");
    let trashed = soft_delete(&paths, &cfg, &mut conn, &id);
    backdate(&trashed);
    seed_vector(&conn, &id);
    assert_eq!(
        count_by_id(&conn, "SELECT COUNT(*) FROM memories WHERE id = ?1", &id),
        1,
        "the soft delete keeps the row"
    );

    let resp = gc(&paths, &cfg, &mut conn);

    assert_eq!(resp.removed, 1, "the aged file was reaped");
    assert_eq!(resp.purged_rows, 1, "its row went with it");
    assert!(!trashed.exists(), "trash file unlinked");
    for (label, sql) in [
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
            "edges",
            "SELECT COUNT(*) FROM edges WHERE (src_kind = 'memory' AND src_id = ?1) \
             OR (dst_kind = 'memory' AND dst_id = ?1)",
        ),
        (
            "memory_vec",
            "SELECT COUNT(*) FROM memory_vec WHERE memory_id = ?1",
        ),
    ] {
        assert_eq!(
            count_by_id(&conn, sql, &id),
            0,
            "{label}: no zombie row after gc"
        );
    }
    assert!(
        trash_ids(&paths, &cfg, &mut conn).is_empty(),
        "the trash listing no longer shows the reaped memory"
    );
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let stats = maintenance::stats::run(&mut ctx, maintenance::stats::Request { repo: None })
        .expect("stats");
    assert_eq!(stats.trashed, 0, "stats.trashed no longer counts it");

    let second = gc(&paths, &cfg, &mut conn);
    assert_eq!(
        (second.removed, second.purged_rows),
        (0, 0),
        "a second sweep finds nothing left"
    );
}

#[test]
fn run_purges_a_zombie_row_whose_trash_file_is_already_gone() {
    // The state every pre-purge `gc` left behind: the file unlinked by an
    // earlier sweep, the `memories` row still soft-deleted and past the
    // window. An upgrade must self-heal on its next sweep.
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_store(&home);
    let id = save_note(&paths, &cfg, &mut conn, "zombie left by an older gc");
    let trashed = soft_delete(&paths, &cfg, &mut conn, &id);
    std::fs::remove_file(&trashed).expect("simulate the earlier sweep's unlink");
    seed_vector(&conn, &id);
    let old =
        comemory::store::memory_row::iso_format(OffsetDateTime::now_utc() - Duration::days(45))
            .expect("old stamp");
    conn.execute(
        "UPDATE memories SET deleted_at = ?1 WHERE id = ?2",
        rusqlite::params![old, id],
    )
    .expect("age deleted_at");
    assert_eq!(
        trash_ids(&paths, &cfg, &mut conn),
        vec![id.clone()],
        "listed as a zombie"
    );

    let resp = gc(&paths, &cfg, &mut conn);

    assert_eq!(resp.removed, 0, "no file left to reap");
    assert_eq!(resp.bytes_freed, 0);
    assert_eq!(resp.purged_rows, 1, "the zombie row is purged");
    assert_eq!(
        count_by_id(&conn, "SELECT COUNT(*) FROM memories WHERE id = ?1", &id),
        0
    );
    assert_eq!(
        count_by_id(
            &conn,
            "SELECT COUNT(*) FROM memory_vec WHERE memory_id = ?1",
            &id
        ),
        0,
        "the zombie's vector row goes with it"
    );
    assert!(trash_ids(&paths, &cfg, &mut conn).is_empty());
}

#[test]
fn run_leaves_live_memories_and_fresh_trash_entries_alone() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_store(&home);
    let live = save_note(&paths, &cfg, &mut conn, "a live memory gc must never touch");
    let fresh = save_note(&paths, &cfg, &mut conn, "deleted today, inside the window");
    let fresh_path = soft_delete(&paths, &cfg, &mut conn, &fresh);
    seed_vector(&conn, &live);
    seed_vector(&conn, &fresh);
    // An old `deleted_at` on a row whose trash file is still on disk is not
    // a zombie: the file's mtime is the clock, and it is fresh.
    let old =
        comemory::store::memory_row::iso_format(OffsetDateTime::now_utc() - Duration::days(45))
            .expect("old stamp");
    conn.execute(
        "UPDATE memories SET deleted_at = ?1 WHERE id = ?2",
        rusqlite::params![old, fresh],
    )
    .expect("age deleted_at");

    let resp = gc(&paths, &cfg, &mut conn);

    assert_eq!((resp.removed, resp.purged_rows), (0, 0));
    assert!(fresh_path.exists(), "fresh trash entry kept");
    assert_eq!(
        count_by_id(
            &conn,
            "SELECT COUNT(*) FROM memories WHERE id = ?1 AND deleted_at IS NULL",
            &live
        ),
        1,
        "live row untouched"
    );
    assert_eq!(
        count_by_id(
            &conn,
            "SELECT COUNT(*) FROM memory_fts WHERE memory_id = ?1",
            &live
        ),
        1,
        "live FTS row untouched"
    );
    assert_eq!(
        count_by_id(
            &conn,
            "SELECT COUNT(*) FROM memory_vec WHERE memory_id = ?1",
            &live
        ),
        1,
        "live vector row untouched"
    );
    assert_eq!(
        count_by_id(
            &conn,
            "SELECT COUNT(*) FROM memory_vec WHERE memory_id = ?1",
            &fresh
        ),
        1,
        "a trash entry inside the window keeps its vector row too"
    );
    assert_eq!(trash_ids(&paths, &cfg, &mut conn), vec![fresh]);
}

#[test]
fn gc_evicts_activity_rows_past_the_window_and_reports_the_count() {
    use comemory::store::activity::{ActivityFilter, NewActivityRow, insert, list};

    let home = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(home.path()).expect("create data dir");
    let mut conn = comemory::store::connection::open(db_path(&home)).expect("open + migrate db");
    let now = OffsetDateTime::now_utc();
    let old = comemory::store::memory_row::iso_format(now - Duration::days(100)).expect("old");
    let fresh = comemory::store::memory_row::iso_format(now - Duration::days(1)).expect("fresh");
    for at in [&old, &fresh] {
        insert(
            &conn,
            &NewActivityRow {
                at,
                command: "save",
                source: "cli",
                actor: None,
                repo: Some("demo"),
                duration_ms: 3,
                ok: true,
                error_code: None,
                summary: None,
                device: None,
                event_id: None,
            },
        )
        .expect("insert activity row");
    }

    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let out = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        maintenance::gc::run(&mut ctx, maintenance::gc::Request {}).expect("gc run")
    };

    assert_eq!(
        out.activity_rows, 1,
        "the row past prune.learning_retention_days is evicted, the fresh one kept"
    );
    let (rows, total) = list(&conn, &ActivityFilter::default(), 0, 0).expect("list activity");
    assert_eq!(total, 1);
    assert_eq!(rows[0].at, fresh);

    let recorded: i64 = conn
        .query_row("SELECT activity_rows FROM gc_runs", [], |r| r.get(0))
        .expect("the sweep recorded its own count");
    assert_eq!(recorded, 1);
}

/// Seed one active generation with a receipt, and one upload abandoned two
/// days ago — the shape AC-10 sweeps over.
fn seed_replica_debris(home: &tempfile::TempDir) {
    std::fs::create_dir_all(home.path()).expect("create data dir");
    let conn = comemory::store::connection::open(db_path(home)).expect("open + migrate db");
    let long_ago =
        comemory::store::memory_row::iso_format(OffsetDateTime::now_utc() - Duration::days(2))
            .expect("stamp");
    let projection = comemory::store::remote_code::Projection {
        files: vec![comemory::store::remote_code::File {
            path: "src/lib.rs".to_string(),
            blob_oid: "aaaa1111".to_string(),
        }],
        symbols: Vec::new(),
        edges: Vec::new(),
    };
    for (id, activate) in [(&"a".repeat(32), true), (&"b".repeat(32), false)] {
        comemory::store::code_generation::record(
            &conn,
            &comemory::store::code_generation::Generation {
                repo: "demo".to_string(),
                generation_id: id.clone(),
                parent_id: None,
                head: "head-1".to_string(),
                mined_commit: None,
                origin: comemory::store::replica_journal::ReplicaOrigin::Sync,
                state: comemory::store::code_generation::State::Staged,
                file_count: 1,
                manifest_digest: "d".repeat(64),
            },
            &long_ago,
        )
        .expect("record");
        comemory::store::remote_code::replace_generation(&conn, "demo", id, &projection)
            .expect("projection");
        if activate {
            comemory::store::code_generation::activate(&conn, "demo", id, &long_ago)
                .expect("activate");
        }
    }
    comemory::store::replica_receipt::record(
        &conn,
        &comemory::store::replica_receipt::Receipt {
            operation_id: "op-20260920-gen00001".to_string(),
            epoch: "epoch-1".to_string(),
            sequence: Some(1),
            disposition: "accepted".to_string(),
            payload_digest: Some("c".repeat(64)),
            reason: None,
        },
        &long_ago,
    )
    .expect("receipt");
    comemory::store::replica_staging::put_part(&conn, "upload-gone", 0, 2, "{}", &long_ago)
        .expect("part");
}

#[test]
fn gc_sweeps_abandoned_uploads_and_spares_the_active_generation() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    seed_replica_debris(&home);
    let cfg = Config::defaults();

    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = maintenance::gc::run(&mut ctx, maintenance::gc::Request {}).expect("gc run");

    assert_eq!(
        resp.staged_rows, 2,
        "one staged part and one staged generation"
    );
    let conn = comemory::store::connection::open(db_path(&home)).expect("open db");
    assert_eq!(
        comemory::store::code_generation::active(&conn, "demo")
            .expect("active")
            .expect("row")
            .generation_id,
        "a".repeat(32),
        "the active generation survived the sweep"
    );
    assert_eq!(
        comemory::store::remote_code::projection(&conn, "demo", &"a".repeat(32))
            .expect("projection")
            .files
            .len(),
        1,
        "so did its projection"
    );
    assert!(
        comemory::store::replica_receipt::lookup(&conn, "op-20260920-gen00001")
            .expect("lookup")
            .is_some(),
        "and every receipt"
    );
    assert_eq!(
        comemory::store::code_generation::by_id(&conn, "demo", &"b".repeat(32)).expect("by_id"),
        None,
        "only the upload that never finished is gone"
    );
}

// ---------------------------------------------------------------------------
// AC-13: `gc` leaves the pulled document cache and the share mapping alone,
// while still doing the sweeping it exists for. Both halves are asserted, or
// "it removed nothing" would also pass for a `gc` that did nothing at all.
// ---------------------------------------------------------------------------

const SHARE_REPO: &str = "Falconiere/comemory";
const SHARE_AT: &str = "2026-09-23T10:00:00Z";

/// Put a real pulled revision and an approval in `conn`.
fn seed_pulled_revision(conn: &rusqlite::Connection) -> String {
    use comemory::domains::documents::document::DocumentFormat;
    use comemory::domains::documents::document::extract::extract;
    use comemory::store::remote_document::{self, Chunk, Link, Revision};

    let path = "docs/guides/cloud-sync.md";
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let bytes = std::fs::read(&file).expect("read the real guide");
    let doc = extract(DocumentFormat::Markdown, &bytes, "doc").expect("real extraction");
    let shared_id = comemory::domains::documents::share::shared_id(SHARE_REPO, path);
    let revision = Revision {
        repo: SHARE_REPO.to_string(),
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
    remote_document::replace_revision(
        conn,
        &revision,
        &chunks,
        &[Link {
            ordinal: 0,
            target: "docs/guides/http-api.md".to_string(),
        }],
        SHARE_AT,
    )
    .expect("hold a pulled revision");
    comemory::store::repository_approval::replace_all(
        conn,
        &[(SHARE_REPO.to_string(), SHARE_REPO.to_string())],
        SHARE_AT,
    )
    .expect("approve");
    shared_id
}

/// An upload part abandoned long enough for the sweep to reclaim it.
fn seed_abandoned_part(conn: &rusqlite::Connection) {
    let long_ago =
        comemory::store::memory_row::iso_format(OffsetDateTime::now_utc() - Duration::hours(48))
            .expect("stamp");
    conn.execute(
        "INSERT INTO replica_staged_part(staging_id, part_index, part_count, bytes, created_at) \
         VALUES ('staging-abandoned', 0, 2, 'partial', ?1)",
        [&long_ago],
    )
    .expect("seed an abandoned part");
}

fn table_count(conn: &rusqlite::Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .expect("count")
}

#[test]
fn gc_sweeps_abandoned_parts_and_touches_no_pulled_document() {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(home.path()).expect("create data dir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let cfg = Config::defaults();
    let mut conn = comemory::store::connection::open(db_path(&home)).expect("open + migrate");
    let shared_id = seed_pulled_revision(&conn);
    seed_abandoned_part(&conn);
    let before = [
        table_count(&conn, "remote_document"),
        table_count(&conn, "remote_document_chunk"),
        table_count(&conn, "remote_document_link"),
        table_count(&conn, "remote_document_fts"),
        table_count(&conn, "repository_approval"),
    ];
    assert!(before.iter().all(|n| *n > 0), "seeded: {before:?}");

    let response = gc(&paths, &cfg, &mut conn);

    assert!(
        response.staged_rows > 0,
        "gc really swept something, so the untouched counts below are not the \
         result of a run that did nothing: {response:?}"
    );
    assert_eq!(
        [
            table_count(&conn, "remote_document"),
            table_count(&conn, "remote_document_chunk"),
            table_count(&conn, "remote_document_link"),
            table_count(&conn, "remote_document_fts"),
            table_count(&conn, "repository_approval"),
        ],
        before,
        "gc has no retention policy for documents, local or pulled"
    );
    assert!(
        comemory::store::remote_document::revision(&conn, SHARE_REPO, &shared_id)
            .expect("read")
            .is_some(),
        "and the revision is still the one this machine holds"
    );
}
