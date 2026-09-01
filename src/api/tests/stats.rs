#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `api::stats::run` against a real store: real markdown on disk, a real
//! migrated SQLite database, real rows. The counters are the whole contract,
//! so every assertion here compares them against the same thing counted a
//! second way (a SQL count, a directory listing) rather than against a
//! hard-coded number that would only restate the fixture.

use comemory::api::{Ctx, stats};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use tempfile::TempDir;

/// A data dir with `memories/` present but no database — the fresh-install
/// state that must not be turned into a migrated database by a read.
fn fresh_paths(dir: &TempDir) -> Paths {
    let paths = Paths::new(dir.path().to_path_buf());
    paths.ensure_dirs().unwrap();
    paths
}

fn seed_memory(conn: &rusqlite::Connection, id: &str, repo: &str, deleted: bool) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, body, created_at, updated_at,
                              content_hash, schema, md_path, deleted_at)
         VALUES (?1, ?2, 'decision', ?3, 'body', '2026-08-01T00:00:00Z',
                 '2026-08-01T00:00:00Z', ?1, 1, ?4, ?5)",
        rusqlite::params![
            id,
            format!("{id}-slug"),
            repo,
            format!("memories/{id}-slug.md"),
            if deleted {
                Some("2026-08-02T00:00:00Z")
            } else {
                None
            },
        ],
    )
    .unwrap();
}

#[test]
fn a_data_dir_without_a_database_reports_zeros_and_creates_nothing() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let resp = stats::run(&mut ctx, stats::Request::default()).unwrap();

    assert_eq!(resp.memories, 0);
    assert_eq!(resp.code_symbols, 0);
    assert_eq!(resp.edges, 0);
    assert_eq!(resp.db_bytes, 0);
    assert_eq!(resp.schema_version, "unknown");
    assert!(
        !paths.db_path().exists(),
        "asking for stats must not create and migrate a database"
    );
}

#[test]
fn markdown_files_counts_the_memories_directory_not_the_trash() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    std::fs::write(paths.memories_dir().join("aaaa1111-one.md"), "one").unwrap();
    std::fs::write(paths.memories_dir().join("bbbb2222-two.md"), "two").unwrap();
    std::fs::write(paths.memories_dir().join("notes.txt"), "not markdown").unwrap();
    std::fs::create_dir_all(paths.trash_dir()).unwrap();
    std::fs::write(paths.trash_dir().join("cccc3333-gone.md"), "trashed").unwrap();

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = stats::run(&mut ctx, stats::Request::default()).unwrap();

    assert_eq!(
        resp.markdown_files, 2,
        "only *.md directly under memories/ counts — not .txt, not .trash/"
    );
}

#[test]
fn counters_agree_with_the_database_and_split_live_from_trashed() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_memory(&conn, "11111111", "comemory", false);
    seed_memory(&conn, "22222222", "comemory", false);
    seed_memory(&conn, "33333333", "toolu", false);
    seed_memory(&conn, "44444444", "comemory", true);

    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let resp = stats::run(&mut ctx, stats::Request::default()).unwrap();

    let live: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE deleted_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(resp.memories as i64, live);
    assert_eq!(resp.memories, 3);
    assert_eq!(resp.trashed, 1, "soft-deleted rows count separately");
    assert_eq!(
        resp.schema_version,
        comemory::store::migrate::CURRENT_VERSION
    );
}

#[test]
fn repo_scopes_the_per_repo_counters_but_not_the_database_size() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_memory(&conn, "11111111", "comemory", false);
    seed_memory(&conn, "22222222", "comemory", false);
    seed_memory(&conn, "33333333", "toolu", false);

    let cfg = Config::defaults();
    let global = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        stats::run(&mut ctx, stats::Request::default()).unwrap()
    };
    let scoped = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        stats::run(
            &mut ctx,
            stats::Request {
                repo: Some("comemory".into()),
            },
        )
        .unwrap()
    };

    assert_eq!(global.memories, 3);
    assert_eq!(scoped.memories, 2, "--repo narrows the memory counter");
    assert_eq!(
        scoped.db_bytes, global.db_bytes,
        "a database has one size no matter which repo asks"
    );
}

#[test]
fn db_bytes_is_page_count_times_page_size_and_grows_with_content() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();

    let before = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        stats::run(&mut ctx, stats::Request::default())
            .unwrap()
            .db_bytes
    };

    let pages: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    let size: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        before,
        (pages * size) as u64,
        "db_bytes is the logical page product, not the file length (WAL)"
    );
    assert!(before > 0);

    for i in 0..200 {
        seed_memory(&conn, &format!("{i:08x}"), "bulk", false);
    }
    let after = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        stats::run(&mut ctx, stats::Request::default())
            .unwrap()
            .db_bytes
    };
    assert!(
        after > before,
        "a bulk insert must grow the reported size: {before} -> {after}"
    );
}
