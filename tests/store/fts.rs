//! Test mirror for `src/store/fts.rs`.

use comemory::store::{connection, fts};
use tempfile::tempdir;

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
        fts::search_memory(&conn, "advisory lock", 10, None, None, (1.0, 3.0)).expect("search");
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
        fts::search_memory(&conn, "advisory lock", 10, None, None, (1.0, 3.0)).expect("search");
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
        fts::search_memory(&conn, "postgres", 10, None, Some("decision"), (1.0, 3.0))
            .expect("filtered search");
    assert_eq!(only_decision.len(), 1, "kind filter must drop the bug row");
    assert_eq!(only_decision[0].memory_id, "dec00001");

    let all =
        fts::search_memory(&conn, "postgres", 10, None, None, (1.0, 3.0)).expect("unfiltered");
    assert_eq!(all.len(), 2, "kind = None must keep both rows");
}

#[test]
fn code_fts_returns_seeded_match() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    let symbol_path = "src/auth/login.rs";
    conn.execute(
        "INSERT INTO code_symbols\
            (id,repo,path,blob_oid,symbol,kind,lang,line_start,line_end,snippet,simhash,indexed_at) \
         VALUES(1,'r',?1,'oid','login::handle','function','rust',1,10,\
                'fn handle() { /* advisory login flow */ }',0,'t')",
        [symbol_path],
    )
    .expect("seed code symbol");

    fts::index_code(
        &conn,
        1,
        "login::handle",
        "fn handle() { /* advisory login flow */ }",
        symbol_path,
    )
    .expect("index code");

    let hits = fts::search_code(&conn, "login", 10).expect("search code");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].symbol_id, 1);
}

#[test]
fn camel_case_path_is_reachable_by_subtoken() {
    // Regression: `path_to_tokens` used to pre-lowercase the path before
    // the identifier tokenizer saw it, destroying the camelCase boundary —
    // `MyComponent.tsx` indexed as `mycomponent` and the query `component`
    // missed. The raw path now goes straight to the tokenizer.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    fts::index_code(
        &conn,
        1,
        "render",
        "fn body without the query term",
        "src/MyComponent.tsx",
    )
    .expect("index code");

    let hits = fts::search_code(&conn, "component", 10).expect("search code");
    assert_eq!(hits.len(), 1, "camelCase path subtoken must match");
    assert_eq!(hits[0].symbol_id, 1);
}

#[test]
fn build_match_query_quotes_and_prefixes_last_term() {
    assert_eq!(
        fts::build_match_query("vec dim mism"),
        r#""vec" "dim" "mism"*"#
    );
    // embedded quotes are stripped, never injected into FTS syntax
    assert_eq!(fts::build_match_query(r#"a"b"#), r#""ab"*"#);
    assert_eq!(fts::build_match_query(""), "");
}

#[test]
fn build_or_query_joins_terms() {
    assert_eq!(
        fts::build_or_query("auth login race"),
        r#""auth" OR "login" OR "race""#
    );
}

#[test]
fn build_subtoken_or_query_expands_identifier_terms() {
    // The colocated whole (`vecdimmismatch`) is deliberately included: it
    // can only add recall for verbatim-identifier mentions, never subtract.
    assert_eq!(
        fts::build_subtoken_or_query("VecDimMismatch"),
        r#""vec" OR "vecdimmismatch" OR "dim" OR "mismatch""#
    );
    // snake_case splits the same way; whole stays colocated after part 1.
    assert_eq!(
        fts::build_subtoken_or_query("dim_guard"),
        r#""dim" OR "dim_guard" OR "guard""#
    );
}

#[test]
fn build_subtoken_or_query_expands_despite_cross_term_part_collisions() {
    // Regression: the old guard compared the aggregate count of distinct
    // non-colocated parts against the term count, so a query whose
    // identifier parts collide with its plain terms ("VecDim vec" → parts
    // vec/dim, 2 terms) was wrongly suppressed to "". The split check is
    // per-term now.
    assert_eq!(
        fts::build_subtoken_or_query("VecDim vec"),
        r#""vec" OR "vecdim" OR "dim""#
    );
    assert_eq!(
        fts::build_subtoken_or_query("DimGuard dim guard"),
        r#""dim" OR "dimguard" OR "guard""#
    );
}

#[test]
fn build_subtoken_or_query_is_empty_when_nothing_splits() {
    // Plain words yield exactly one part per term — no expansion possible,
    // so the builder signals that with an empty expression.
    assert_eq!(fts::build_subtoken_or_query("kubernetes"), "");
    assert_eq!(fts::build_subtoken_or_query("oauth login race"), "");
    assert_eq!(fts::build_subtoken_or_query(""), "");
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
    let strict =
        fts::search_memory(&conn, "VecDimMismatch", 10, None, None, (1.0, 3.0)).expect("strict");
    assert!(strict.is_empty(), "strict phrase tier must miss prose body");
    // …but the subtoken OR tier finds it.
    let hits = fts::search_memory_subtokens(&conn, "VecDimMismatch", 10, None, None, (1.0, 3.0))
        .expect("subtokens");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "mem1");
}

#[test]
fn term_count_matches_what_the_builders_quote() {
    // `" foo` is 2 raw whitespace terms but only 1 sanitized term: the
    // lone quote sanitizes to empty and is dropped, exactly as the MATCH
    // builders do.
    assert_eq!(fts::term_count(r#"" foo"#), 1);
    assert_eq!(fts::term_count("a b c"), 3);
    assert_eq!(fts::term_count(""), 0);
    assert_eq!(fts::term_count("\"\" \"\""), 0);
}

#[test]
fn builders_clamp_to_first_32_sanitized_terms() {
    let query = (0..40)
        .map(|i| format!("t{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let strict = fts::build_match_query(&query);
    // 32 terms × 2 quotes each; the prefix `*` lands on the 32nd kept term.
    assert_eq!(strict.matches('"').count(), 64);
    assert!(strict.ends_with(r#""t31"*"#), "got: {strict}");
    assert!(!strict.contains("t32"));
    let relaxed = fts::build_or_query(&query);
    assert_eq!(relaxed.matches(" OR ").count(), 31);
    assert!(!relaxed.contains('*'), "relaxed tier must not prefix-match");
    assert_eq!(fts::term_count(&query), 32);
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

#[test]
fn empty_and_quote_only_queries_return_empty_without_error() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    assert!(fts::search_memory(&conn, "", 10, None, None, (1.0, 3.0))
        .expect("empty query")
        .is_empty());
    // A quote-only query sanitizes to an empty MATCH expression; it must
    // come back empty rather than surfacing an FTS5 syntax error.
    assert!(
        fts::search_memory(&conn, "\"\"", 10, None, None, (1.0, 3.0))
            .expect("quote-only query")
            .is_empty()
    );
    assert!(fts::search_code(&conn, "", 10)
        .expect("empty code query")
        .is_empty());
    assert!(fts::search_code(&conn, "\"\"", 10)
        .expect("quote-only code query")
        .is_empty());
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
fn code_symbol_match_outranks_snippet_match() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    fts::index_code(
        &conn,
        1,
        "unrelated_name",
        "calls handle_login somewhere",
        "src other rs",
    )
    .expect("index 1");
    fts::index_code(
        &conn,
        2,
        "handle_login",
        "fn body without the query term",
        "src auth rs",
    )
    .expect("index 2");

    let hits = fts::search_code(&conn, "handle_login", 10).expect("search code");
    assert_eq!(hits.len(), 2);
    assert_eq!(
        hits[0].symbol_id, 2,
        "symbol-column hit must outrank snippet hit"
    );
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
