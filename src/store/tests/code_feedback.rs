#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration coverage for `src/store/code_feedback.rs` — the
//! `code_feedback` / code-tagged `feedback_events` CRUD and identity
//! lookups moved out of `stats::code_feedback`. Driven through the real
//! [`StatsDb`] + `record_code_with_provenance` path, which still owns the
//! chunk-to-parent resolution and the transaction boundary, rather than
//! calling the `pub(crate)` store helpers with a bare connection.

use comemory::config::paths::Paths;
use comemory::stats::code_feedback::record_code_with_provenance;
use comemory::stats::feedback::PROV_MANUAL;
use comemory::stats::sqlite::StatsDb;
use comemory::store::code_row::{self, CodeSymbolRow};
use tempfile::TempDir;

/// Open a [`StatsDb`] over a fresh `comemory.db` in a tempdir.
fn open_db() -> (StatsDb, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::new(tmp.path());
    let db = StatsDb::open(paths.stats_db()).expect("open stats db");
    (db, tmp)
}

/// Insert one real `code_symbols` row via the production writer.
fn seed_row(
    conn: &rusqlite::Connection,
    repo: &str,
    path: &str,
    symbol: &str,
    parent_id: Option<i64>,
) -> i64 {
    code_row::insert(
        conn,
        &CodeSymbolRow {
            repo,
            path,
            blob_oid: "oid",
            symbol,
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 10,
            snippet: "fn body() {}",
            simhash: 0,
            parent_id,
        },
    )
    .expect("insert code symbol")
}

#[test]
fn absent_code_feedback_row_precedes_first_record() {
    let (db, _tmp) = open_db();
    let row: Option<i64> = db
        .conn()
        .query_row(
            "SELECT used_count FROM code_feedback WHERE repo = 'demo' AND path = 'a.rs' \
              AND symbol = 'alpha'",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(row, None, "no counter row before any feedback is recorded");
}

#[test]
fn record_code_with_provenance_resolves_identity_and_writes_counter() {
    let (mut db, _tmp) = open_db();
    let id = seed_row(db.conn(), "demo", "a.rs", "alpha", None);
    record_code_with_provenance(&mut db, "q-20260610-aabbccd1", &[id], &[], PROV_MANUAL)
        .expect("record");

    let conn = db.conn();
    let (used, _irrelevant): (i64, i64) = conn
        .query_row(
            "SELECT used_count, irrelevant_count FROM code_feedback \
              WHERE repo = 'demo' AND path = 'a.rs' AND symbol = 'alpha'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("counter row keyed by identity, not rowid");
    assert_eq!(used, 1);

    let target_kind: String = conn
        .query_row(
            "SELECT target_kind FROM feedback_events WHERE memory_id = ?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .expect("event row keeps the text-encoded rowid");
    assert_eq!(target_kind, "code");
}

#[test]
fn unknown_symbol_id_errors_loudly_rather_than_defaulting() {
    // Contrast with the "row absent means zero" counter idiom: an absent
    // `code_symbols` row is NOT a default — resolve_identity must error
    // naming the id, since there is nothing to attribute the verdict to.
    let (mut db, _tmp) = open_db();
    let err =
        record_code_with_provenance(&mut db, "q-20260610-aabbccd1", &[9_999], &[], PROV_MANUAL)
            .expect_err("unknown id must error");
    assert!(
        err.to_string().contains("9999"),
        "error names the id: {err}"
    );
}

#[test]
fn chunk_with_vanished_parent_falls_back_to_own_identity() {
    // Exercises the split between `own_identity` and `parent_identity`:
    // a dangling parent_id must degrade to the chunk's own identity rather
    // than erroring or writing a parent-keyed row.
    let (mut db, _tmp) = open_db();
    let parent = seed_row(db.conn(), "demo", "a.rs", "alpha", None);
    let chunk = seed_row(db.conn(), "demo", "a.rs", "alpha#1", Some(parent));
    db.conn()
        .execute("DELETE FROM code_symbols WHERE id = ?1", [parent])
        .expect("vanish parent row");

    record_code_with_provenance(&mut db, "q-20260611-aabbccd1", &[chunk], &[], PROV_MANUAL)
        .expect("record against orphaned chunk");
    let used: i64 = db
        .conn()
        .query_row(
            "SELECT used_count FROM code_feedback \
              WHERE repo = 'demo' AND path = 'a.rs' AND symbol = 'alpha#1'",
            [],
            |r| r.get(0),
        )
        .expect("own-identity row");
    assert_eq!(
        used, 1,
        "dangling parent_id degrades to the chunk's own identity"
    );
}
