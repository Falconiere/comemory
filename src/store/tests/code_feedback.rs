#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration coverage for `src/store/code_feedback.rs` — the
//! `code_feedback` / code-tagged `feedback_events` CRUD and identity
//! lookups moved out of `domains::learning::code_feedback`. Driven through the real
//! [`StatsDb`] + `record_code_with_provenance` path, which still owns the
//! chunk-to-parent resolution and the transaction boundary, rather than
//! calling the `pub(crate)` store helpers with a bare connection.

use comemory::config::paths::Paths;
use comemory::domains::learning::code_feedback::record_code_with_provenance;
use comemory::domains::learning::telemetry::StatsDb;
use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::utilities::telemetry::PROV_MANUAL;
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

#[test]
fn both_identity_reads_carry_the_rows_blob_oid_as_the_code_version() {
    use comemory::store::code_feedback::{own_identity, parent_identity};
    let (db, _tmp) = open_db();
    let parent = code_row::insert(
        db.conn(),
        &CodeSymbolRow {
            repo: "demo",
            path: "a.rs",
            blob_oid: "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
            symbol: "alpha",
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 40,
            snippet: "fn alpha() {}",
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("parent");
    let chunk = seed_row(db.conn(), "demo", "a.rs", "alpha#1", Some(parent));

    let (own, parent_id) = own_identity(db.conn(), chunk).expect("own").expect("row");
    assert_eq!(
        (own.symbol.as_str(), own.version.as_deref(), parent_id),
        ("alpha#1", Some("oid"), Some(parent))
    );
    let resolved = parent_identity(db.conn(), parent)
        .expect("parent")
        .expect("row");
    assert_eq!(
        (resolved.symbol.as_str(), resolved.version.as_deref()),
        ("alpha", Some("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")),
        "the version is the resolved row's own, not the chunk's"
    );
}

/// Read one `code_feedback` counter row back as `(used, irrelevant, last_used)`.
fn counter_row(conn: &rusqlite::Connection, symbol: &str) -> (i64, i64, Option<String>) {
    conn.query_row(
        "SELECT used_count, irrelevant_count, last_used FROM code_feedback \
          WHERE repo = 'demo' AND path = 'a.rs' AND symbol = ?1",
        [symbol],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .expect("counter row")
}

#[test]
fn upserts_keep_the_latest_last_used_and_dismissals_never_touch_it() {
    use comemory::store::code_feedback::{SymbolIdentity, upsert_irrelevant, upsert_used};
    let (db, _tmp) = open_db();
    let conn = db.conn();
    let sym = |name: &str| SymbolIdentity {
        repo: "demo".to_string(),
        path: "a.rs".to_string(),
        symbol: name.to_string(),
        version: None,
    };

    // Dismissed first: the row exists with no last_used, and a later use
    // stamps it.
    let alpha = sym("alpha");
    upsert_irrelevant(conn, &alpha).expect("dismiss");
    assert_eq!(counter_row(conn, "alpha"), (0, 1, None));
    upsert_used(conn, &alpha, "2026-06-10T00:00:00Z").expect("use");
    assert_eq!(
        counter_row(conn, "alpha"),
        (1, 1, Some("2026-06-10T00:00:00Z".to_string()))
    );

    // A dismissal after a use leaves the stamp alone; an out-of-order older use
    // bumps the count but cannot move the stamp backwards.
    upsert_irrelevant(conn, &alpha).expect("dismiss again");
    upsert_used(conn, &alpha, "2026-06-01T00:00:00Z").expect("older use");
    assert_eq!(
        counter_row(conn, "alpha"),
        (2, 2, Some("2026-06-10T00:00:00Z".to_string()))
    );
    upsert_used(conn, &alpha, "2026-06-20T00:00:00Z").expect("newer use");
    assert_eq!(
        counter_row(conn, "alpha"),
        (3, 2, Some("2026-06-20T00:00:00Z".to_string()))
    );

    // A fresh symbol used first inserts used_count = 1 and the stamp.
    upsert_used(conn, &sym("beta"), "2026-06-05T00:00:00Z").expect("first use");
    assert_eq!(
        counter_row(conn, "beta"),
        (1, 0, Some("2026-06-05T00:00:00Z".to_string()))
    );
}
