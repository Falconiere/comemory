#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/store/fts_memory.rs` — the memory-leg FTS5
//! ladder (strict, relaxed, subtoken, expanded) and its BM25 weighting.

use comemory::store::{CreatedWindow, connection, fts, fts_memory};
use tempfile::tempdir;

/// Default `memory_fts` BM25 weights `(body, tags)`.
const WEIGHTS: (f32, f32) = (1.0, 3.0);

/// Seed one `query_expansions` row (tier-4 input).
fn seed_expansion(conn: &rusqlite::Connection, term: &str, expansion: &str, support: i64) {
    conn.execute(
        "INSERT INTO query_expansions(term, expansion, support, last_mined) \
         VALUES (?1, ?2, ?3, '2026-06-09T12:00:00Z')",
        rusqlite::params![term, expansion, support],
    )
    .expect("seed expansion");
}

#[test]
fn bm25_returns_seeded_match() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('mem1','m','note','h','postgres advisory locks for migration','t','t','m.md')",
        [],
    )
    .expect("seed memory");

    fts::index_memory(
        &conn,
        "mem1",
        "postgres advisory locks for migration",
        "db,postgres",
    )
    .expect("index");

    let hits = fts_memory::search_memory(
        &conn,
        "advisory lock",
        10,
        None,
        None,
        CreatedWindow::default(),
        WEIGHTS,
    )
    .expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "mem1");
}

#[test]
fn search_memory_skips_soft_deleted() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,deleted_at,md_path) \
         VALUES('mem1','m','note','h','postgres advisory locks for migration','t','t','t','m.md')",
        [],
    )
    .expect("seed memory");

    fts::index_memory(
        &conn,
        "mem1",
        "postgres advisory locks for migration",
        "db,postgres",
    )
    .expect("index");

    let hits = fts_memory::search_memory(
        &conn,
        "advisory lock",
        10,
        None,
        None,
        CreatedWindow::default(),
        WEIGHTS,
    )
    .expect("search");
    assert!(
        hits.is_empty(),
        "soft-deleted memories must not appear in FTS results, got {hits:?}",
        hits = hits
            .iter()
            .map(|h| h.memory_id.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn kind_filter_restricts_memory_search() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path)
         VALUES ('dec00001','a','decision','h1','postgres advisory locks chosen','t','t','m/1.md'),
                ('bug00001','b','bug','h2','postgres pool exhaustion observed','t','t','m/2.md');",
    )
    .expect("seed");
    fts::index_memory(&conn, "dec00001", "postgres advisory locks chosen", "").expect("index");
    fts::index_memory(&conn, "bug00001", "postgres pool exhaustion observed", "").expect("index");

    let only_decision = fts_memory::search_memory(
        &conn,
        "postgres",
        10,
        None,
        Some("decision"),
        CreatedWindow::default(),
        WEIGHTS,
    )
    .expect("filtered search");
    assert_eq!(only_decision.len(), 1, "kind filter must drop the bug row");
    assert_eq!(only_decision[0].memory_id, "dec00001");

    let all = fts_memory::search_memory(
        &conn,
        "postgres",
        10,
        None,
        None,
        CreatedWindow::default(),
        WEIGHTS,
    )
    .expect("unfiltered");
    assert_eq!(all.len(), 2, "kind = None must keep both rows");
}

#[test]
fn relaxed_search_matches_on_any_term() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('mem1','m','note','h','the oauth refresh race condition','t','t','m.md')",
        [],
    )
    .expect("seed memory");
    fts::index_memory(&conn, "mem1", "the oauth refresh race condition", "").expect("index");

    // Strict AND of all three terms fails ('login' is absent)…
    let strict = fts_memory::search_memory(
        &conn,
        "oauth login race",
        10,
        None,
        None,
        CreatedWindow::default(),
        WEIGHTS,
    )
    .expect("strict");
    assert!(
        strict.is_empty(),
        "strict AND must miss when a term is absent"
    );
    // …but the relaxed OR variant still finds the memory.
    let relaxed = fts_memory::search_memory_relaxed(
        &conn,
        "oauth login race",
        10,
        None,
        None,
        CreatedWindow::default(),
        WEIGHTS,
    )
    .expect("relaxed");
    assert_eq!(relaxed.len(), 1);
    assert_eq!(relaxed[0].memory_id, "mem1");
}

#[test]
fn subtoken_search_matches_prose_parts_of_identifier() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    let body = "embedder returned wrong dim mismatch against the vec table";
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('mem1','m','note','h',?1,'t','t','m.md')",
        [body],
    )
    .expect("seed memory");
    fts::index_memory(&conn, "mem1", body, "").expect("index");

    // Strict tier misses: the quoted identifier becomes a *phrase* over
    // its subtokens, which the prose body has non-consecutively…
    let strict = fts_memory::search_memory(
        &conn,
        "VecDimMismatch",
        10,
        None,
        None,
        CreatedWindow::default(),
        WEIGHTS,
    )
    .expect("strict");
    assert!(strict.is_empty(), "strict phrase tier must miss prose body");
    // …but the subtoken OR tier finds it.
    let hits = fts_memory::search_memory_subtokens(
        &conn,
        "VecDimMismatch",
        10,
        None,
        None,
        CreatedWindow::default(),
        WEIGHTS,
    )
    .expect("subtokens");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "mem1");
}

#[test]
fn expanded_search_reaches_memory_containing_only_the_expansion_term() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    let body = "the vecdimmismatch guard fired again";
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('mem1','m','note','h',?1,'t','t','m.md')",
        [body],
    )
    .expect("seed memory");
    fts::index_memory(&conn, "mem1", body, "").expect("index");
    seed_expansion(&conn, "sizing", "vecdimmismatch", 2);

    let hits = fts_memory::search_memory_expanded(
        &conn,
        "sizing",
        10,
        None,
        None,
        CreatedWindow::default(),
        WEIGHTS,
    )
    .expect("expanded search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "mem1");

    // No applicable expansion (different term) -> empty without touching FTS.
    let none = fts_memory::search_memory_expanded(
        &conn,
        "kubernetes",
        10,
        None,
        None,
        CreatedWindow::default(),
        WEIGHTS,
    )
    .expect("no expansion");
    assert!(none.is_empty());
}

#[test]
fn tag_match_outranks_body_match() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash)
         VALUES ('aaaa0001','a','note','d','f',3,1,'h1','postgres mentioned once in body',
                 '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/1.md',1),
                ('aaaa0002','b','note','d','f',3,1,'h2','completely unrelated body text',
                 '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/2.md',2);
         INSERT INTO memory_fts(memory_id, body, tags)
         VALUES ('aaaa0001','postgres mentioned once in body',''),
                ('aaaa0002','completely unrelated body text','postgres');",
    )
    .expect("seed");
    let hits = fts_memory::search_memory(
        &conn,
        "postgres",
        10,
        None,
        None,
        CreatedWindow::default(),
        WEIGHTS,
    )
    .expect("search");
    assert_eq!(
        hits[0].memory_id, "aaaa0002",
        "tag hit must outrank body hit"
    );
}

#[test]
fn bm25_weights_parameter_flips_column_priority() {
    // One memory matches the query only in its body, the other only in its
    // tags. Tags-heavy weights (the (1.0, 3.0) default) must rank the tag
    // hit first; body-heavy weights (3.0, 1.0) must flip the order.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash)
         VALUES ('bodyhit1','a','note','d','f',3,1,'h1','postgres mentioned once in body',
                 '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/1.md',1),
                ('taghit01','b','note','d','f',3,1,'h2','completely unrelated body text',
                 '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/2.md',2);
         INSERT INTO memory_fts(memory_id, body, tags)
         VALUES ('bodyhit1','postgres mentioned once in body',''),
                ('taghit01','completely unrelated body text','postgres');",
    )
    .expect("seed");

    let tags_heavy = fts_memory::search_memory(
        &conn,
        "postgres",
        10,
        None,
        None,
        CreatedWindow::default(),
        WEIGHTS,
    )
    .expect("search");
    assert_eq!(tags_heavy.len(), 2);
    assert_eq!(
        tags_heavy[0].memory_id, "taghit01",
        "tags-heavy weights must rank the tag hit first"
    );

    let body_heavy = fts_memory::search_memory(
        &conn,
        "postgres",
        10,
        None,
        None,
        CreatedWindow::default(),
        (3.0, 1.0),
    )
    .expect("search");
    assert_eq!(body_heavy.len(), 2);
    assert_eq!(
        body_heavy[0].memory_id, "bodyhit1",
        "body-heavy weights must rank the body hit first"
    );
}

/// Seed one indexed memory with an explicit `created_at`, so the
/// created-date window has something with a known timestamp to filter.
fn seed_dated(conn: &rusqlite::Connection, id: &str, body: &str, created: &str) {
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES(?1,'s','note','h',?2,?3,?3,'m.md')",
        rusqlite::params![id, body, created],
    )
    .expect("seed dated memory");
    fts::index_memory(conn, id, body, "").expect("index");
}

/// Ids surviving a cutoff-only (`--until`-shaped) window.
fn ids_before(conn: &rusqlite::Connection, cutoff: &str) -> Vec<String> {
    ids_in_window(
        conn,
        CreatedWindow {
            since: None,
            cutoff: Some(cutoff),
        },
    )
}

/// Search "postgres" through the strict tier under `window`, returning ids.
fn ids_in_window(conn: &rusqlite::Connection, window: CreatedWindow<'_>) -> Vec<String> {
    fts_memory::search_memory(conn, "postgres", 10, None, None, window, WEIGHTS)
        .expect("search")
        .into_iter()
        .map(|h| h.memory_id)
        .collect()
}

/// Three matching memories created on 2026-03-01, -10 and -20.
fn open_dated_corpus() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    for (id, created) in [
        ("early001", "2026-03-01T12:00:00Z"),
        ("mid00001", "2026-03-10T12:00:00Z"),
        ("late0001", "2026-03-20T12:00:00Z"),
    ] {
        seed_dated(&conn, id, "postgres note", created);
    }
    (dir, conn)
}

#[test]
fn created_window_bounds_are_inclusive_and_exclude_outside() {
    let (_dir, conn) = open_dated_corpus();

    let unbounded = ids_in_window(&conn, CreatedWindow::default());
    assert_eq!(
        unbounded.len(),
        3,
        "no window sees every row: {unbounded:?}"
    );

    let cutoff_only = ids_before(&conn, "2026-03-10T12:00:00Z");
    assert!(
        cutoff_only.contains(&"mid00001".to_string()),
        "cutoff is inclusive of an exactly-equal timestamp: {cutoff_only:?}"
    );
    assert!(
        !cutoff_only.contains(&"late0001".to_string()),
        "row created after the cutoff must be excluded: {cutoff_only:?}"
    );

    let since_only = ids_in_window(
        &conn,
        CreatedWindow {
            since: Some("2026-03-10T12:00:00Z"),
            cutoff: None,
        },
    );
    assert!(
        since_only.contains(&"mid00001".to_string()),
        "since is inclusive of an exactly-equal timestamp: {since_only:?}"
    );
    assert!(
        !since_only.contains(&"early001".to_string()),
        "row created before since must be excluded: {since_only:?}"
    );

    let both = ids_in_window(
        &conn,
        CreatedWindow {
            since: Some("2026-03-05T00:00:00Z"),
            cutoff: Some("2026-03-15T00:00:00Z"),
        },
    );
    assert_eq!(both, ["mid00001"], "both bounds narrow to the middle row");
}

#[test]
fn created_window_compares_across_mixed_timestamp_precision() {
    // AC-8. Rows store whole seconds; the bounds carry 9 fractional digits
    // — the shape a `--until <bare date>` day-edge produces.
    //
    // `edge0001` is the discriminating case: stored "2026-06-09T23:59:59Z"
    // against cutoff "2026-06-09T23:59:59.999999999Z". A raw string
    // compare sorts the row AFTER the cutoff ('Z' > '.' at the fractional
    // position) and would drop it, while chronologically it is inside the
    // window. Only `datetime()` normalization keeps it.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    seed_dated(
        &conn,
        "mixed001",
        "postgres pooling",
        "2026-06-09T00:00:00Z",
    );
    seed_dated(
        &conn,
        "edge0001",
        "postgres sharding",
        "2026-06-09T23:59:59Z",
    );

    let same_day = ids_before(&conn, "2026-06-09T23:59:59.999999999Z");
    assert!(
        same_day.contains(&"mixed001".to_string()),
        "end-of-day cutoff must include a row stored without fractional seconds: {same_day:?}"
    );
    assert!(
        same_day.contains(&"edge0001".to_string()),
        "a row on the cutoff's own second must survive the fractional bound \
         (string compare would drop it): {same_day:?}"
    );

    let previous_day = ids_before(&conn, "2026-06-08T23:59:59.999999999Z");
    assert!(
        previous_day.is_empty(),
        "previous-day cutoff must exclude both rows: {previous_day:?}"
    );
}
