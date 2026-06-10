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

    let hits = fts::search_memory(&conn, "advisory lock", 10, None).expect("search");
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

    let hits = fts::search_memory(&conn, "advisory lock", 10, None).expect("search");
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

    let path_tokens = fts::path_to_tokens(symbol_path);
    fts::index_code(
        &conn,
        1,
        "login::handle",
        "fn handle() { /* advisory login flow */ }",
        &path_tokens,
    )
    .expect("index code");

    let hits = fts::search_code(&conn, "login", 10).expect("search code");
    assert_eq!(hits.len(), 1);
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
    let hits = fts::search_memory(&conn, "postgres", 10, None).expect("search");
    assert_eq!(
        hits[0].memory_id, "aaaa0002",
        "tag hit must outrank body hit"
    );
}

#[test]
fn empty_and_quote_only_queries_return_empty_without_error() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    assert!(fts::search_memory(&conn, "", 10, None)
        .expect("empty query")
        .is_empty());
    // A quote-only query sanitizes to an empty MATCH expression; it must
    // come back empty rather than surfacing an FTS5 syntax error.
    assert!(fts::search_memory(&conn, "\"\"", 10, None)
        .expect("quote-only query")
        .is_empty());
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
    let strict = fts::search_memory(&conn, "oauth login race", 10, None).expect("strict");
    assert!(
        strict.is_empty(),
        "strict AND must miss when a term is absent"
    );
    // …but the relaxed OR variant still finds the memory.
    let relaxed = fts::search_memory_relaxed(&conn, "oauth login race", 10, None).expect("relaxed");
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

#[test]
fn path_to_tokens_lowercases_and_splits_non_alnum() {
    assert_eq!(
        fts::path_to_tokens("src/Foo/Bar_baz.rs"),
        "src foo bar baz rs"
    );
    assert_eq!(
        fts::path_to_tokens("crates/CoreLib/mod-utils.ts"),
        "crates corelib mod utils ts"
    );
    assert_eq!(fts::path_to_tokens(""), "");
}
