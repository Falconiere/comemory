//! Test mirror for `src/store/fts_memory.rs` — the memory-leg FTS5
//! ladder (strict, relaxed, subtoken, expanded) and its BM25 weighting.

use comemory::store::{connection, fts, fts_memory};
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

    let hits =
        fts_memory::search_memory(&conn, "advisory lock", 10, None, None, WEIGHTS).expect("search");
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

    let hits =
        fts_memory::search_memory(&conn, "advisory lock", 10, None, None, WEIGHTS).expect("search");
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

    let only_decision =
        fts_memory::search_memory(&conn, "postgres", 10, None, Some("decision"), WEIGHTS)
            .expect("filtered search");
    assert_eq!(only_decision.len(), 1, "kind filter must drop the bug row");
    assert_eq!(only_decision[0].memory_id, "dec00001");

    let all =
        fts_memory::search_memory(&conn, "postgres", 10, None, None, WEIGHTS).expect("unfiltered");
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
    let strict = fts_memory::search_memory(&conn, "oauth login race", 10, None, None, WEIGHTS)
        .expect("strict");
    assert!(
        strict.is_empty(),
        "strict AND must miss when a term is absent"
    );
    // …but the relaxed OR variant still finds the memory.
    let relaxed =
        fts_memory::search_memory_relaxed(&conn, "oauth login race", 10, None, None, WEIGHTS)
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
    let strict = fts_memory::search_memory(&conn, "VecDimMismatch", 10, None, None, WEIGHTS)
        .expect("strict");
    assert!(strict.is_empty(), "strict phrase tier must miss prose body");
    // …but the subtoken OR tier finds it.
    let hits =
        fts_memory::search_memory_subtokens(&conn, "VecDimMismatch", 10, None, None, WEIGHTS)
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

    let hits = fts_memory::search_memory_expanded(&conn, "sizing", 10, None, None, WEIGHTS)
        .expect("expanded search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "mem1");

    // No applicable expansion (different term) -> empty without touching FTS.
    let none = fts_memory::search_memory_expanded(&conn, "kubernetes", 10, None, None, WEIGHTS)
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
    let hits =
        fts_memory::search_memory(&conn, "postgres", 10, None, None, WEIGHTS).expect("search");
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

    let tags_heavy =
        fts_memory::search_memory(&conn, "postgres", 10, None, None, WEIGHTS).expect("search");
    assert_eq!(tags_heavy.len(), 2);
    assert_eq!(
        tags_heavy[0].memory_id, "taghit01",
        "tags-heavy weights must rank the tag hit first"
    );

    let body_heavy =
        fts_memory::search_memory(&conn, "postgres", 10, None, None, (3.0, 1.0)).expect("search");
    assert_eq!(body_heavy.len(), 2);
    assert_eq!(
        body_heavy[0].memory_id, "bodyhit1",
        "body-heavy weights must rank the body hit first"
    );
}
