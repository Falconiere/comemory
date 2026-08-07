#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api.rs`.
//!
//! Proves the two `Ctx` constructors keep their contract: [`Ctx::lazy`]
//! never opens `comemory.db` until [`Ctx::conn`] is first called, and
//! [`Ctx::borrowed`] hands back the caller's own connection rather than
//! opening a second one.

use comemory::api::Ctx;
use comemory::config::{Config, Paths};
use rusqlite::Connection;

use crate::test_common as common;

#[test]
fn lazy_ctx_opens_the_db_only_on_first_conn_call() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    // `ensure_dirs` creates the memories/trash/index tree (and, as a side
    // effect of `create_dir_all`, the data-dir root itself) — but never
    // touches `comemory.db`.
    paths.ensure_dirs().expect("ensure_dirs");
    let cfg = Config::defaults();
    let db_path = paths.db_path();

    assert!(
        !db_path.exists(),
        "comemory.db must not exist before Ctx::lazy is even constructed"
    );

    let mut ctx = Ctx::lazy(&paths, &cfg);
    assert!(
        !db_path.exists(),
        "constructing a lazy Ctx must not open the database"
    );

    let conn = ctx.conn().expect("conn opens the database lazily");
    assert!(
        db_path.exists(),
        "conn() must have opened comemory.db on first use"
    );

    let memory_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("trivial query against the migrated schema");
    assert_eq!(memory_count, 0);
}

#[test]
fn borrowed_ctx_reuses_the_callers_connection() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    let cfg = Config::defaults();

    let mut conn = Connection::open_in_memory().expect("open in-memory connection");
    conn.execute("CREATE TABLE ctx_probe (id INTEGER PRIMARY KEY)", [])
        .expect("create a table through the original handle");

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let handle = ctx.conn().expect("conn returns the borrowed connection");

    // An in-memory database is private to its own connection: a second,
    // freshly opened connection to `:memory:` would see an empty schema.
    // Finding the probe table through `ctx.conn()` proves it is literally
    // the same connection object the caller created it on, not a new one.
    let table_exists: i64 = handle
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'ctx_probe'",
            [],
            |row| row.get(0),
        )
        .expect("query sqlite_master through Ctx::conn()");
    assert_eq!(
        table_exists, 1,
        "Ctx::conn() must return the exact connection the caller passed in"
    );
}
