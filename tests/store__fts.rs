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

/// Default `code_fts` BM25 weights `(symbol, snippet, path_tokens)`.
const CODE_WEIGHTS: (f32, f32, f32) = (2.0, 1.0, 1.5);

/// Insert one `code_symbols` row plus its `code_fts` sibling. The
/// `code_symbols` row is required since `search_code` joins it for the
/// repo / lang filters.
fn seed_code_symbol(
    conn: &rusqlite::Connection,
    id: i64,
    repo: &str,
    lang: &str,
    symbol: &str,
    snippet: &str,
    path: &str,
) {
    conn.execute(
        "INSERT INTO code_symbols\
            (id,repo,path,blob_oid,symbol,kind,lang,line_start,line_end,snippet,simhash,indexed_at) \
         VALUES(?1,?2,?3,'oid',?4,'function',?5,1,10,?6,0,'t')",
        rusqlite::params![id, repo, path, symbol, lang, snippet],
    )
    .expect("seed code symbol");
    fts::index_code(conn, id, symbol, snippet, path).expect("index code");
}

#[test]
fn code_fts_returns_seeded_match() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    seed_code_symbol(
        &conn,
        1,
        "r",
        "rust",
        "login::handle",
        "fn handle() { /* advisory login flow */ }",
        "src/auth/login.rs",
    );

    let hits = fts::search_code(&conn, "login", 10, None, None, CODE_WEIGHTS).expect("search code");
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
    seed_code_symbol(
        &conn,
        1,
        "r",
        "typescript",
        "render",
        "fn body without the query term",
        "src/MyComponent.tsx",
    );

    let hits =
        fts::search_code(&conn, "component", 10, None, None, CODE_WEIGHTS).expect("search code");
    assert_eq!(hits.len(), 1, "camelCase path subtoken must match");
    assert_eq!(hits[0].symbol_id, 1);
}

#[test]
fn repo_and_lang_filters_restrict_code_search() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_code_symbol(
        &conn,
        1,
        "frontend",
        "rust",
        "config::parse",
        "fn parse() { /* config */ }",
        "src/config.rs",
    );
    seed_code_symbol(
        &conn,
        2,
        "backend",
        "python",
        "parse_config",
        "def parse_config(): pass",
        "tools/config.py",
    );

    let rust_only = fts::search_code(&conn, "config", 10, None, Some("rust"), CODE_WEIGHTS)
        .expect("lang filter");
    assert_eq!(rust_only.len(), 1, "lang filter must drop the python row");
    assert_eq!(rust_only[0].symbol_id, 1);

    let backend_only = fts::search_code(&conn, "config", 10, Some("backend"), None, CODE_WEIGHTS)
        .expect("repo filter");
    assert_eq!(
        backend_only.len(),
        1,
        "repo filter must drop the frontend row"
    );
    assert_eq!(backend_only[0].symbol_id, 2);

    let all = fts::search_code(&conn, "config", 10, None, None, CODE_WEIGHTS).expect("unfiltered");
    assert_eq!(all.len(), 2, "no filter must keep both rows");
}

#[test]
fn code_bm25_weights_parameter_flips_column_priority() {
    // Symbol 1 matches the query only in its snippet; symbol 2 only in its
    // symbol name. Symbol-heavy weights (the default) must rank 2 first;
    // snippet-heavy weights must flip the order.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_code_symbol(
        &conn,
        1,
        "r",
        "rust",
        "unrelated_name",
        "calls handle_login somewhere",
        "src other rs",
    );
    seed_code_symbol(
        &conn,
        2,
        "r",
        "rust",
        "handle_login",
        "fn body without the query term",
        "src auth rs",
    );

    let symbol_heavy =
        fts::search_code(&conn, "handle_login", 10, None, None, CODE_WEIGHTS).expect("search");
    assert_eq!(symbol_heavy.len(), 2);
    assert_eq!(
        symbol_heavy[0].symbol_id, 2,
        "symbol-heavy weights must rank the symbol-name hit first"
    );

    let snippet_heavy =
        fts::search_code(&conn, "handle_login", 10, None, None, (1.0, 6.0, 1.5)).expect("search");
    assert_eq!(snippet_heavy.len(), 2);
    assert_eq!(
        snippet_heavy[0].symbol_id, 1,
        "snippet-heavy weights must rank the snippet hit first"
    );
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
