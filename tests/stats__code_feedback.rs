//! Tests for [`comemory::stats::code_feedback`].
//!
//! Code-side sibling of `tests/stats/feedback.rs`: counter upsert semantics
//! (first insert → 1, conflict → +1, last_used refresh) are exercised
//! through `record_code_with_provenance`, the only src/ writer. Counter
//! rows are keyed by the stable (repo, path, symbol) identity — NOT the
//! `code_symbols` rowid, which SQLite recycles after a re-index
//! purge+reinsert. Provenance rows land in `feedback_events` with
//! `target_kind = 'code'` and the symbol rowid text-encoded into the
//! `memory_id` column — the column name is a memory-era wart the writer
//! documents rather than hides.

use comemory::config::paths::Paths;
use comemory::stats::code_feedback::record_code_with_provenance;
use comemory::stats::sqlite::StatsDb;
use comemory::store::code_row::{self, CodeSymbolRow};

#[path = "common/mod.rs"]
mod common;

/// Open a [`StatsDb`] in a fresh sandbox, returning the guard with it.
fn open_db() -> (common::runner::Sandbox, StatsDb) {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    let db = StatsDb::open(paths.stats_db()).expect("open");
    (sb, db)
}

/// Insert one real `code_symbols` row via the production writer and
/// return its rowid — feedback now resolves ids against live rows, so
/// every test seeds the symbols it scores. A `Some` `parent_id` makes the
/// row a cAST chunk child of an earlier-seeded parent.
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

/// Top-level (non-chunk) convenience wrapper over [`seed_row`].
fn seed_symbol(conn: &rusqlite::Connection, repo: &str, path: &str, symbol: &str) -> i64 {
    seed_row(conn, repo, path, symbol, None)
}

/// Fetch the identity-keyed counter row for one symbol.
fn counter_row(
    conn: &rusqlite::Connection,
    repo: &str,
    path: &str,
    symbol: &str,
) -> Option<(i64, i64, Option<String>)> {
    conn.query_row(
        "SELECT used_count, irrelevant_count, last_used FROM code_feedback \
          WHERE repo = ?1 AND path = ?2 AND symbol = ?3",
        rusqlite::params![repo, path, symbol],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .ok()
}

#[test]
fn used_counter_inserts_then_increments_and_refreshes_last_used() {
    let (_sb, mut db) = open_db();
    let id = seed_symbol(db.conn(), "demo", "a.rs", "alpha");
    record_code_with_provenance(&mut db, "q-20260610-aabbccd1", &[id], &[]).expect("first record");
    let (used, _, last) = counter_row(db.conn(), "demo", "a.rs", "alpha").expect("row");
    assert_eq!(used, 1, "first insert seeds used_count = 1");
    assert!(last.is_some_and(|l| !l.is_empty()), "insert sets last_used");

    // Backdate last_used so the conflict path's refresh is observable
    // without sleeping between the two records.
    db.conn()
        .execute(
            "UPDATE code_feedback SET last_used = '2000-01-01T00:00:00Z' \
              WHERE repo = 'demo' AND path = 'a.rs' AND symbol = 'alpha'",
            [],
        )
        .expect("backdate last_used");
    record_code_with_provenance(&mut db, "q-20260610-aabbccd2", &[id], &[]).expect("second record");
    let (used, _, last) = counter_row(db.conn(), "demo", "a.rs", "alpha").expect("row");
    let last = last.expect("last_used set");
    assert_eq!(used, 2, "conflict bumps used_count");
    assert!(
        last.as_str() > "2000-01-01T00:00:00Z",
        "conflict refreshes last_used, got {last}"
    );
}

#[test]
fn irrelevant_counter_inserts_then_increments_without_touching_last_used() {
    let (_sb, mut db) = open_db();
    let id = seed_symbol(db.conn(), "demo", "b.rs", "beta");
    for qid in ["q-20260610-aabbccd1", "q-20260610-aabbccd2"] {
        record_code_with_provenance(&mut db, qid, &[], &[id]).expect("record");
    }
    let (used, irrelevant, last) = counter_row(db.conn(), "demo", "b.rs", "beta").expect("row");
    assert_eq!(used, 0);
    assert_eq!(irrelevant, 2, "insert seeds 1, conflict bumps to 2");
    assert!(last.is_none(), "a dismissal is not a use");
}

#[test]
fn record_code_with_provenance_writes_code_tagged_events_and_counters() {
    let (_sb, mut db) = open_db();
    let used_id = seed_symbol(db.conn(), "demo", "a.rs", "alpha");
    let irrelevant_id = seed_symbol(db.conn(), "demo", "b.rs", "beta");
    record_code_with_provenance(&mut db, "q-20260610-aabbccdd", &[used_id], &[irrelevant_id])
        .expect("record");

    let conn = db.conn();
    let events: i64 = conn
        .query_row(
            "SELECT count(*) FROM feedback_events \
              WHERE query_id = 'q-20260610-aabbccdd' AND target_kind = 'code'",
            [],
            |r| r.get(0),
        )
        .expect("events");
    assert_eq!(events, 2, "every code event must carry target_kind='code'");
    // Events keep the text-encoded ROWID in the memory_id column (the
    // documented column-name wart): point-in-time telemetry, never
    // re-joined for ranking.
    let verdict: String = conn
        .query_row(
            "SELECT verdict FROM feedback_events WHERE memory_id = ?1",
            [used_id.to_string()],
            |r| r.get(0),
        )
        .expect("used verdict");
    assert_eq!(verdict, "used");
    let verdict: String = conn
        .query_row(
            "SELECT verdict FROM feedback_events WHERE memory_id = ?1",
            [irrelevant_id.to_string()],
            |r| r.get(0),
        )
        .expect("irrelevant verdict");
    assert_eq!(verdict, "irrelevant");
    // Counters land under the stable identity, not the rowid.
    let (used, _, _) = counter_row(conn, "demo", "a.rs", "alpha").expect("used counter");
    assert_eq!(used, 1);
    // The memory-side counter table must stay untouched by code feedback.
    let memory_rows: i64 = conn
        .query_row("SELECT count(*) FROM feedback", [], |r| r.get(0))
        .expect("memory feedback rows");
    assert_eq!(memory_rows, 0, "code feedback must not touch `feedback`");
}

#[test]
fn record_code_with_provenance_errors_loudly_on_unknown_symbol_id() {
    // Asymmetry with the query-id warn path, by design: a vanished symbol
    // id leaves nothing to attribute the verdict to (the rowid may already
    // name an unrelated symbol), so the write must fail naming the id and
    // the all-or-nothing transaction must leave no partial rows behind.
    let (_sb, mut db) = open_db();
    let live = seed_symbol(db.conn(), "demo", "a.rs", "alpha");
    let err = record_code_with_provenance(&mut db, "q-20260610-aabbccdd", &[live, 9_999], &[])
        .expect_err("unknown symbol id must error");
    let msg = err.to_string();
    assert!(
        msg.contains("9999"),
        "error must name the offending id, got: {msg}"
    );
    let (events, counters): (i64, i64) = db
        .conn()
        .query_row(
            "SELECT (SELECT count(*) FROM feedback_events),
                    (SELECT count(*) FROM code_feedback)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("counts");
    assert_eq!(
        (events, counters),
        (0, 0),
        "failed batch must roll back the live id's rows too"
    );
}

#[test]
fn record_code_with_provenance_errors_on_schema_drift() {
    // Schema drift: if the `code_feedback` table is missing entirely, the
    // write must surface the SQLite error rather than swallow it, and the
    // all-or-nothing transaction must leave no event row behind.
    let (_sb, mut db) = open_db();
    let id = seed_symbol(db.conn(), "demo", "a.rs", "alpha");
    db.conn()
        .execute("DROP TABLE code_feedback", [])
        .expect("drop code_feedback table");

    let err = record_code_with_provenance(&mut db, "q-20260610-aabbccdd", &[id], &[])
        .expect_err("record must error when code_feedback table is missing");
    let msg = err.to_string();
    assert!(
        msg.contains("code_feedback"),
        "error should mention 'code_feedback', got: {msg}"
    );
    let events: i64 = db
        .conn()
        .query_row("SELECT count(*) FROM feedback_events", [], |r| r.get(0))
        .expect("count events");
    assert_eq!(events, 0, "failed batch must not leave a partial event row");
}

/// Regression for the final-integration review's live demo: `code_symbols`
/// rowids are recycled by the per-file purge+reinsert on re-index, so
/// feedback keyed by rowid was re-attributed to whatever symbol inherited
/// the number. Identity-keyed rows must survive the re-index attached to
/// the SAME symbol, and the rowid's new owner must inherit nothing.
#[test]
fn feedback_survives_reindex_rowid_recycling_without_misattribution() {
    let (_sb, mut db) = open_db();
    let _alpha = seed_symbol(db.conn(), "demo", "f.rs", "alpha");
    let beta = seed_symbol(db.conn(), "demo", "f.rs", "beta");
    record_code_with_provenance(&mut db, "q-20260610-aabbccd1", &[beta], &[]).expect("record beta");
    record_code_with_provenance(&mut db, "q-20260610-aabbccd2", &[beta], &[])
        .expect("record beta again");

    // Re-index the file with a new symbol `gamma` inserted ABOVE the
    // others: the purge frees the rowids and the reinsert hands beta's
    // old number to a DIFFERENT symbol (the production re-index path uses
    // exactly this purge+reinsert sequence).
    code_row::purge_file_symbols(db.conn(), "demo", "f.rs").expect("purge");
    let _gamma = seed_symbol(db.conn(), "demo", "f.rs", "gamma");
    let _alpha = seed_symbol(db.conn(), "demo", "f.rs", "alpha");
    let new_beta = seed_symbol(db.conn(), "demo", "f.rs", "beta");
    let inheritor: String = db
        .conn()
        .query_row(
            "SELECT symbol FROM code_symbols WHERE id = ?1",
            [beta],
            |r| r.get(0),
        )
        .expect("recycled rowid must be live again");
    assert_ne!(
        inheritor, "beta",
        "fixture must reproduce the rowid recycling (beta's old id changes owner)"
    );

    let (used, _, _) = counter_row(db.conn(), "demo", "f.rs", "beta").expect("beta row");
    assert_eq!(used, 2, "beta must keep its feedback across the re-index");
    for other in ["gamma", "alpha"] {
        assert!(
            counter_row(db.conn(), "demo", "f.rs", other).is_none(),
            "{other} must NOT inherit beta's feedback via a recycled rowid"
        );
    }

    // New feedback against beta's NEW rowid accumulates onto the same
    // identity row.
    record_code_with_provenance(&mut db, "q-20260610-aabbccd3", &[new_beta], &[])
        .expect("record new beta");
    let (used, _, _) = counter_row(db.conn(), "demo", "f.rs", "beta").expect("beta row");
    assert_eq!(used, 3, "post-re-index feedback joins the same identity");
}

/// PR #7 follow-up: a verdict recorded against a CHUNK rowid (`parent_id`
/// set) must land under the PARENT's identity. The chunk's own `name#n`
/// key is one the COALESCE-to-parent scoring join in
/// `retrieval::code_prior::signals` can never match, so a chunk-keyed
/// counter row would be silently inert feedback.
#[test]
fn feedback_against_chunk_id_resolves_to_parent_identity() {
    let (_sb, mut db) = open_db();
    let parent = seed_symbol(db.conn(), "demo", "a.rs", "alpha");
    let chunk = seed_row(db.conn(), "demo", "a.rs", "alpha#1", Some(parent));
    record_code_with_provenance(&mut db, "q-20260611-aabbccd1", &[chunk], &[])
        .expect("record chunk verdict");
    let (used, _, _) = counter_row(db.conn(), "demo", "a.rs", "alpha").expect("parent-keyed row");
    assert_eq!(used, 1, "chunk verdict must land under the parent symbol");
    assert!(
        counter_row(db.conn(), "demo", "a.rs", "alpha#1").is_none(),
        "no inert chunk-keyed counter row may be written"
    );
    // The provenance event still carries the CHUNK rowid verbatim —
    // identity resolution is a counter-key concern only.
    let event_target: String = db
        .conn()
        .query_row(
            "SELECT memory_id FROM feedback_events WHERE query_id = 'q-20260611-aabbccd1'",
            [],
            |r| r.get(0),
        )
        .expect("event row");
    assert_eq!(event_target, chunk.to_string());

    // A verdict against the parent's own id increments the SAME row.
    record_code_with_provenance(&mut db, "q-20260611-aabbccd2", &[parent], &[])
        .expect("record parent verdict");
    let (used, _, _) = counter_row(db.conn(), "demo", "a.rs", "alpha").expect("parent-keyed row");
    assert_eq!(used, 2, "parent and chunk verdicts share one identity row");
}

/// Degraded case: a chunk whose parent row vanished (raced re-index
/// delete) keeps its OWN identity — consistent with
/// `retrieval::code_rerank::coalesce` and the COALESCE fallback in
/// `retrieval::code_prior::signals`.
#[test]
fn chunk_with_vanished_parent_falls_back_to_own_identity() {
    let (_sb, mut db) = open_db();
    let parent = seed_symbol(db.conn(), "demo", "a.rs", "alpha");
    let chunk = seed_row(db.conn(), "demo", "a.rs", "alpha#1", Some(parent));
    db.conn()
        .execute("DELETE FROM code_symbols WHERE id = ?1", [parent])
        .expect("vanish parent row");
    record_code_with_provenance(&mut db, "q-20260611-aabbccd1", &[chunk], &[])
        .expect("record against orphaned chunk");
    let (used, _, _) = counter_row(db.conn(), "demo", "a.rs", "alpha#1").expect("own-identity row");
    assert_eq!(
        used, 1,
        "dangling parent_id degrades to the chunk's own identity"
    );
}
