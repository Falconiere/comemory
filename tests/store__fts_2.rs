//! Test mirror for `src/store/fts.rs` (part 2 — expanded-query and
//! error-classifier tests).

use comemory::store::{connection, fts};
use tempfile::tempdir;

/// Seed one `query_expansions` row.
fn seed_expansion(conn: &rusqlite::Connection, term: &str, expansion: &str, support: i64) {
    conn.execute(
        "INSERT INTO query_expansions(term, expansion, support, last_mined) \
         VALUES (?1, ?2, ?3, '2026-06-09T12:00:00Z')",
        rusqlite::params![term, expansion, support],
    )
    .expect("seed expansion");
}

#[test]
fn empty_and_quote_only_queries_return_empty_without_error() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");

    /// Default `code_fts` BM25 weights `(symbol, snippet, path_tokens)`.
    const CODE_WEIGHTS: (f32, f32, f32) = (2.0, 1.0, 1.5);

    let results = fts::search_memory(&conn, "", 10, None, None, (1.0, 3.0));
    assert!(results.expect("empty query").is_empty());
    // A quote-only query sanitizes to an empty MATCH expression; it must
    // come back empty rather than surfacing an FTS5 syntax error.
    let results = fts::search_memory(&conn, "\"\"", 10, None, None, (1.0, 3.0));
    assert!(results.expect("quote-only query").is_empty());
    let results = fts::search_code(&conn, "", 10, None, None, CODE_WEIGHTS);
    assert!(results.expect("empty code query").is_empty());
    let results = fts::search_code(&conn, "\"\"", 10, None, None, CODE_WEIGHTS);
    assert!(results.expect("quote-only code query").is_empty());
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
    let strict =
        fts::search_memory(&conn, "oauth login race", 10, None, None, (1.0, 3.0)).expect("strict");
    assert!(
        strict.is_empty(),
        "strict AND must miss when a term is absent"
    );
    // …but the relaxed OR variant still finds the memory.
    let relaxed = fts::search_memory_relaxed(&conn, "oauth login race", 10, None, None, (1.0, 3.0))
        .expect("relaxed");
    assert_eq!(relaxed.len(), 1);
    assert_eq!(relaxed[0].memory_id, "mem1");
}

#[test]
fn build_expanded_or_query_ors_terms_with_mined_expansions() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    // Three mappings for `sizing`: support 3 wins, then the support-2 tie
    // breaks alphabetically; the per-term cap (2) drops `guard`.
    seed_expansion(&conn, "sizing", "vecdimmismatch", 3);
    seed_expansion(&conn, "sizing", "dim", 2);
    seed_expansion(&conn, "sizing", "guard", 2);
    let expr = fts::build_expanded_or_query(&conn, "sizing problem").expect("build");
    assert_eq!(
        expr,
        r#""sizing" OR "vecdimmismatch" OR "dim" OR "problem""#
    );
}

#[test]
fn build_expanded_or_query_lowercases_the_lookup_key() {
    // Mining stores tokenized (lowercase) terms; the raw-cased query term
    // must still find its mapping.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_expansion(&conn, "sizing", "vecdimmismatch", 2);
    let expr = fts::build_expanded_or_query(&conn, "Sizing").expect("build");
    assert_eq!(expr, r#""sizing" OR "vecdimmismatch""#);
}

#[test]
fn build_expanded_or_query_folds_diacritics_like_mining_does() {
    // Regression: mining writes tokenizer-normalized keys (diacritic
    // folded — a failed query "Café" is stored under "cafe"), but the
    // lookup used to be a bare to_lowercase(), so the mapping could never
    // fire for the very query shape that produced it. The lookup now goes
    // through the same tokenization; the base OR-token is folded too, so
    // it matches the (folded) FTS index tokens.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_expansion(&conn, "cafe", "espresso", 2);
    let expr = fts::build_expanded_or_query(&conn, "Café").expect("build");
    assert_eq!(expr, r#""cafe" OR "espresso""#);
}

#[test]
fn build_expanded_or_query_splits_identifier_terms_like_mining_does() {
    // Mining tokenizes identifiers into their non-colocated parts, so a
    // mapping mined from a failed `VecDim` query is keyed `dim` (or
    // `vec`), never the raw `vecdim`. The lookup must split identically
    // for the mapping to fire.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_expansion(&conn, "dim", "embedding", 2);
    let expr = fts::build_expanded_or_query(&conn, "VecDim").expect("build");
    assert_eq!(expr, r#""vec" OR "dim" OR "embedding""#);
}

#[test]
fn build_expanded_or_query_is_empty_below_min_support() {
    // Support 1 is an anecdote, not a rule: no applicable expansion means
    // an empty expression so the tier short-circuits.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_expansion(&conn, "sizing", "vecdimmismatch", 1);
    let expr = fts::build_expanded_or_query(&conn, "sizing").expect("build");
    assert_eq!(expr, "");
    let empty = fts::build_expanded_or_query(&conn, "").expect("build empty");
    assert_eq!(empty, "");
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

    let hits = fts::search_memory_expanded(&conn, "sizing", 10, None, None, (1.0, 3.0))
        .expect("expanded search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "mem1");

    // No applicable expansion (different term) -> empty without touching FTS.
    let none = fts::search_memory_expanded(&conn, "kubernetes", 10, None, None, (1.0, 3.0))
        .expect("no expansion");
    assert!(none.is_empty());
}

/// Pins the SQLite error-text contract behind `is_fts5_parse_error`: a
/// genuinely malformed MATCH expression must classify as a parse error
/// (so search downgrades to empty results), and a non-parse error must
/// not. If a future SQLite/rusqlite bump changes the error wording,
/// this fails at CI time instead of silently swallowing real errors.
#[test]
fn fts5_parse_error_classifier_matches_real_sqlite_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");

    // Bare operator: real FTS5 parse error ("fts5: syntax error near ...").
    // An unbalanced quote yields the different "unterminated string" text
    // the classifier deliberately does NOT match — the builders strip
    // quotes, so that shape is unreachable from built queries.
    let parse_err = conn
        .prepare("SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'AND'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .expect_err("malformed MATCH must error");
    assert!(
        comemory::store::fts::is_fts5_parse_error(&parse_err),
        "classifier must accept a real FTS5 parse error, got: {parse_err}"
    );

    // Missing table: ordinary SQLite error, must NOT classify as parse.
    let other_err = conn
        .prepare("SELECT count(*) FROM no_such_table")
        .expect_err("missing table must error");
    assert!(
        !comemory::store::fts::is_fts5_parse_error(&other_err),
        "classifier must reject non-parse errors, got: {other_err}"
    );
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
    let hits = fts::search_memory(&conn, "postgres", 10, None, None, (1.0, 3.0)).expect("search");
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
        fts::search_memory(&conn, "postgres", 10, None, None, (1.0, 3.0)).expect("search");
    assert_eq!(tags_heavy.len(), 2);
    assert_eq!(
        tags_heavy[0].memory_id, "taghit01",
        "tags-heavy weights must rank the tag hit first"
    );

    let body_heavy =
        fts::search_memory(&conn, "postgres", 10, None, None, (3.0, 1.0)).expect("search");
    assert_eq!(body_heavy.len(), 2);
    assert_eq!(
        body_heavy[0].memory_id, "bodyhit1",
        "body-heavy weights must rank the body hit first"
    );
}
