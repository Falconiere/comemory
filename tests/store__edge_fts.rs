//! Behavior tests for [`comemory::store::edge_fts`] — the FTS5 triplet
//! index over `edges`.
//!
//! Every test runs against a real migrated `comemory.db`. Edges are seeded
//! by raw INSERT in the exact forms the production writers emit: bare 8-hex
//! memory ids on both sides of a relation edge (`memory_row::
//! insert_relation_edges`), bare `<repo>:<path>[:<symbol>]` reference
//! targets (`cross_link::extract_and_emit`), and `file:`-prefixed
//! `co_activated` targets (`coactivate::harvest`).

use comemory::store::edge_fts;
use rusqlite::{Connection, params};

/// A freshly migrated `comemory.db` inside a tempdir.
fn open_db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// Insert one live memory row with an explicit `kind` and `slug` — the two
/// fields the memory endpoint renderer concatenates.
fn seed_memory(conn: &Connection, id: &str, kind: &str, slug: &str) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, content_hash, body,
                              created_at, updated_at, md_path)
         VALUES(?1, ?2, ?3, 'demo', 'f', 'h', 'body',
                '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z', 'm/x.md')",
        params![id, slug, kind],
    )
    .expect("seed memory");
}

/// Insert one edge with the default weight of 1.
fn seed_edge(conn: &Connection, src_kind: &str, src: &str, rel: &str, dst_kind: &str, dst: &str) {
    conn.execute(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5, '2026-07-01T00:00:00Z')",
        params![src_kind, src, dst_kind, dst, rel],
    )
    .expect("seed edge");
}

/// Memory→memory relation edge: bare 8-hex ids on both sides.
fn seed_relation(conn: &Connection, src: &str, rel: &str, dst: &str) {
    seed_edge(conn, "memory", src, rel, "memory", dst);
}

/// Insert one `source_roots` row.
fn seed_source_root(conn: &Connection, id: &str, canonical_path: &str) {
    conn.execute(
        "INSERT INTO source_roots(id, canonical_path, kind, repo, status, created_at, updated_at)
         VALUES(?1, ?2, 'dir', NULL, 'active', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z')",
        params![id, canonical_path],
    )
    .expect("seed source_roots");
}

/// Insert one `source_files` row owned by `source_id`.
fn seed_source_file(conn: &Connection, id: &str, source_id: &str, relative_path: &str) {
    conn.execute(
        "INSERT INTO source_files(id, source_id, relative_path, classification, size, mtime,
                                   sha256, status, error, created_at, updated_at)
         VALUES(?1, ?2, ?3, 'document', 10, 0, 'h', 'indexed', NULL,
                '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z')",
        params![id, source_id, relative_path],
    )
    .expect("seed source_files");
}

/// Insert one `documents` row owned by `source_file_id`, `repo` optional.
fn seed_document(
    conn: &Connection,
    id: &str,
    source_file_id: &str,
    title: &str,
    repo: Option<&str>,
) {
    conn.execute(
        "INSERT INTO documents(id, source_file_id, title, repo, revision_hash,
                                created_at, updated_at)
         VALUES(?1, ?2, ?3, ?4, 'h', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z')",
        params![id, source_file_id, title, repo],
    )
    .expect("seed documents");
}

/// Every indexed triplet, one `|`-joined line per row in rowid order —
/// the full observable content of the index, for identity comparisons.
fn dump(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT rowid, src_text, rel_text, dst_text, src_kind, src_id,
                    rel, dst_kind, dst_id, weight
               FROM edge_fts ORDER BY rowid",
        )
        .expect("prepare dump");
    stmt.query_map([], |r| {
        Ok(format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, String>(8)?,
            r.get::<_, i64>(9)?,
        ))
    })
    .expect("dump query")
    .collect::<Result<Vec<_>, _>>()
    .expect("dump rows")
}

/// Search and return the matched hits, discarding the `has_more` flag.
fn hits(conn: &Connection, query: &str) -> Vec<edge_fts::EdgeFtsHit> {
    edge_fts::search_edges(conn, query, 20, 0)
        .expect("search")
        .0
}

/// The `(src_id, rel, dst_id)` triple of every hit, in returned order.
fn keys(conn: &Connection, query: &str) -> Vec<(String, String, String)> {
    hits(conn, query)
        .into_iter()
        .map(|h| (h.src_id, h.rel, h.dst_id))
        .collect()
}

// ─── AC-2: rendering ─────────────────────────────────────────────────────

#[test]
fn memory_endpoints_render_as_kind_plus_slug_on_both_sides() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_memory(&conn, "bbbb2222", "note", "queue-design-v1");
    seed_relation(&conn, "aaaa1111", "supersedes", "bbbb2222");

    assert_eq!(edge_fts::refresh(&mut conn).expect("refresh"), 1);

    let found = hits(&conn, "supersedes queue design");
    assert_eq!(found.len(), 1, "kind+slug terms should match on both sides");
    assert_eq!(found[0].src_text, "decision queue-design-v2");
    assert_eq!(found[0].dst_text, "note queue-design-v1");
    // The raw edge rides along in the UNINDEXED payload columns.
    assert_eq!(found[0].src_id, "aaaa1111");
    assert_eq!(found[0].dst_id, "bbbb2222");
    assert_eq!(found[0].rel, "supersedes");
    assert_eq!(found[0].weight, 1);
    // Score is the negated BM25: higher is better, so a hit is positive.
    assert!(
        found[0].score > 0.0,
        "score {} not positive",
        found[0].score
    );
}

#[test]
fn symbol_reference_matches_on_both_path_and_symbol_subtokens() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "discovery", "tokenizer-notes");
    seed_edge(
        &conn,
        "memory",
        "aaaa1111",
        "references_symbol",
        "symbol",
        "demo:src/store/tokenizer.rs:split_text",
    );
    edge_fts::refresh(&mut conn).expect("refresh");

    // A bare `references_*` target carries no kind prefix, so it is indexed
    // verbatim and the identifier tokenizer splits `:` `/` `.` and `_`.
    let found = hits(&conn, "tokenizer");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].dst_text, "demo:src/store/tokenizer.rs:split_text");
    // Path directory term and symbol subtoken both reach the same row.
    assert_eq!(keys(&conn, "store tokenizer").len(), 1, "path subtokens");
    assert_eq!(keys(&conn, "split text").len(), 1, "symbol subtokens");
}

#[test]
fn co_activated_file_target_is_stripped_to_its_path_terms() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "bug", "startup-crash");
    seed_edge(
        &conn,
        "memory",
        "aaaa1111",
        "co_activated",
        "file",
        "file:demo:src/main.rs",
    );
    edge_fts::refresh(&mut conn).expect("refresh");

    let found = hits(&conn, "main");
    assert_eq!(found.len(), 1);
    // The `file:` prefix is dropped: the kind already rides in `dst_kind`,
    // and leaving it in would plant `file` in nearly every triplet.
    assert_eq!(found[0].dst_text, "demo:src/main.rs");
    assert_eq!(found[0].dst_kind, "file");
    assert_eq!(found[0].dst_id, "file:demo:src/main.rs");
    assert!(
        hits(&conn, "file").is_empty(),
        "stripped prefix should not be indexed as a term"
    );
}

#[test]
fn document_endpoint_renders_as_identity_path_and_title() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_source_root(&conn, "src1", "/tmp/src1");
    seed_source_file(&conn, "file1", "src1", "guide.md");
    seed_document(
        &conn,
        "doc111100000000000000000000",
        "file1",
        "Comemory Guide",
        Some("demo"),
    );
    seed_edge(
        &conn,
        "memory",
        "aaaa1111",
        "references_document",
        "document",
        "doc111100000000000000000000",
    );
    edge_fts::refresh(&mut conn).expect("refresh");

    let found = hits(&conn, "guide");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].dst_text, "demo:guide.md \u{2014} Comemory Guide");
    assert_eq!(found[0].dst_kind, "document");
    assert_eq!(found[0].dst_id, "doc111100000000000000000000");
    assert!(
        hits(&conn, "doc111100000000000000000000").is_empty(),
        "the raw 128-bit hex id must never be indexed as a term"
    );
}

#[test]
fn document_endpoint_without_repo_renders_path_and_title_only() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_source_root(&conn, "src2", "/tmp/src2");
    seed_source_file(&conn, "file2", "src2", "notes/todo.md");
    seed_document(
        &conn,
        "doc22220000000000000000000",
        "file2",
        "Todo List",
        None,
    );
    seed_edge(
        &conn,
        "memory",
        "aaaa1111",
        "references_document",
        "document",
        "doc22220000000000000000000",
    );
    edge_fts::refresh(&mut conn).expect("refresh");

    let found = hits(&conn, "todo");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].dst_text, "notes/todo.md \u{2014} Todo List");
}

#[test]
fn source_endpoint_renders_as_its_canonical_path() {
    let (_dir, mut conn) = open_db();
    seed_source_root(&conn, "src3", "/home/user/notes");
    seed_source_file(&conn, "file3", "src3", "guide.md");
    seed_edge(&conn, "file", "file3", "member_of_source", "source", "src3");
    edge_fts::refresh(&mut conn).expect("refresh");

    let found = hits(&conn, "notes");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].dst_text, "/home/user/notes");
    assert_eq!(found[0].dst_kind, "source");
    assert_eq!(found[0].dst_id, "src3");
}

#[test]
fn dangling_document_endpoint_falls_back_to_its_bare_id() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_edge(
        &conn,
        "memory",
        "aaaa1111",
        "references_document",
        "document",
        "ghostdoc00",
    );
    edge_fts::refresh(&mut conn).expect("refresh");

    let found = hits(&conn, "ghostdoc00");
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].dst_text, "ghostdoc00",
        "a document with no live row must fall back to its bare id"
    );
}

#[test]
fn dangling_memory_endpoint_falls_back_to_its_bare_id() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    // Frontmatter may name a memory that was never saved; `save` writes the
    // edge anyway and every consumer joins on live rows.
    seed_relation(&conn, "aaaa1111", "supersedes", "ffff9999");
    // A soft-deleted endpoint renders the same way: not a live memory.
    seed_memory(&conn, "cccc3333", "note", "retired-idea");
    conn.execute(
        "UPDATE memories SET deleted_at = '2026-07-02T00:00:00Z' WHERE id = 'cccc3333'",
        [],
    )
    .expect("soft-delete");
    seed_relation(&conn, "aaaa1111", "derived_from", "cccc3333");

    edge_fts::refresh(&mut conn).expect("refresh");

    let dangling = hits(&conn, "ffff9999");
    assert_eq!(dangling.len(), 1);
    assert_eq!(dangling[0].dst_text, "ffff9999");
    let deleted = hits(&conn, "cccc3333");
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].dst_text, "cccc3333");
}

#[test]
fn rel_text_tokenizes_so_derived_from_matches_the_split_verb() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "pattern", "retry-loop");
    seed_memory(&conn, "bbbb2222", "decision", "backoff-policy");
    seed_relation(&conn, "aaaa1111", "derived_from", "bbbb2222");
    edge_fts::refresh(&mut conn).expect("refresh");

    // `derived_from` indexes as `derived` + `from` (+ the colocated whole),
    // so the natural two-word phrasing reaches it.
    assert_eq!(keys(&conn, "derived from").len(), 1);
    assert_eq!(keys(&conn, "derived_from").len(), 1);
    assert_eq!(keys(&conn, "derived from backoff").len(), 1);
}

// ─── AC-3: query semantics ───────────────────────────────────────────────

#[test]
fn strict_and_tier_requires_every_term() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_memory(&conn, "bbbb2222", "note", "queue-design-v1");
    seed_relation(&conn, "aaaa1111", "supersedes", "bbbb2222");
    // A second edge sharing only the rel, so an AND query on rel + slug
    // cannot be satisfied by the rel alone.
    seed_memory(&conn, "cccc3333", "bug", "cache-stampede");
    seed_memory(&conn, "dddd4444", "bug", "cache-warmup");
    seed_relation(&conn, "cccc3333", "supersedes", "dddd4444");
    edge_fts::refresh(&mut conn).expect("refresh");

    let found = keys(&conn, "supersedes cache");
    assert_eq!(found.len(), 1, "strict AND should exclude the queue edge");
    assert_eq!(found[0].0, "cccc3333");
}

#[test]
fn or_fallback_answers_a_query_the_strict_tier_cannot() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_memory(&conn, "bbbb2222", "note", "queue-design-v1");
    seed_relation(&conn, "aaaa1111", "supersedes", "bbbb2222");
    edge_fts::refresh(&mut conn).expect("refresh");

    // Strict AND finds nothing (no triplet mentions `parquet`); the word-OR
    // fallback still surfaces the edge on the term that does exist.
    assert!(
        hits(&conn, "parquet").is_empty(),
        "control: the unrelated term must not match on its own"
    );
    let found = keys(&conn, "supersedes parquet");
    assert_eq!(found.len(), 1, "OR fallback should recover the edge");
    assert_eq!(found[0].0, "aaaa1111");
}

#[test]
fn tied_scores_break_deterministically_and_repeat_across_runs() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "dddd4444", "note", "queue-design");
    // Three sources with byte-identical rendered text: BM25 ties, so the
    // `(src_id, rel, dst_id)` tie-break is the only thing ordering them.
    for id in ["cccc3333", "aaaa1111", "bbbb2222"] {
        seed_memory(&conn, id, "decision", "queue-design");
        seed_relation(&conn, id, "supersedes", "dddd4444");
    }
    edge_fts::refresh(&mut conn).expect("refresh");

    let first = keys(&conn, "decision queue design");
    let src_ids: Vec<&str> = first.iter().map(|k| k.0.as_str()).collect();
    assert_eq!(src_ids, ["aaaa1111", "bbbb2222", "cccc3333"]);
    for _ in 0..4 {
        assert_eq!(keys(&conn, "decision queue design"), first);
    }
}

#[test]
fn pagination_reports_has_more_until_the_last_page() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "dddd4444", "note", "queue-design");
    for id in ["aaaa1111", "bbbb2222", "cccc3333"] {
        seed_memory(&conn, id, "decision", "queue-design");
        seed_relation(&conn, id, "supersedes", "dddd4444");
    }
    edge_fts::refresh(&mut conn).expect("refresh");

    let (page1, more1) = edge_fts::search_edges(&conn, "queue design", 2, 0).expect("page 1");
    assert_eq!(page1.len(), 2);
    assert!(more1, "a third edge remains");
    let (page2, more2) = edge_fts::search_edges(&conn, "queue design", 2, 2).expect("page 2");
    assert_eq!(page2.len(), 1);
    assert!(!more2, "the last page must not claim more");
    // Pages partition the result set — no repeats across the boundary.
    assert_eq!(page1[0].src_id, "aaaa1111");
    assert_eq!(page1[1].src_id, "bbbb2222");
    assert_eq!(page2[0].src_id, "cccc3333");
}

#[test]
fn a_malformed_or_empty_query_yields_an_empty_page_not_an_error() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_memory(&conn, "bbbb2222", "note", "queue-design-v1");
    seed_relation(&conn, "aaaa1111", "supersedes", "bbbb2222");
    edge_fts::refresh(&mut conn).expect("refresh");

    assert!(hits(&conn, "   ").is_empty());
    let (found, more) = edge_fts::search_edges(&conn, "queue", 0, 0).expect("k = 0");
    assert!(found.is_empty() && !more, "k = 0 short-circuits");
}

// ─── AC-9: determinism + refresh semantics ───────────────────────────────

#[test]
fn two_consecutive_refreshes_produce_identical_contents() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_memory(&conn, "bbbb2222", "note", "queue-design-v1");
    seed_relation(&conn, "aaaa1111", "supersedes", "bbbb2222");
    seed_edge(
        &conn,
        "memory",
        "aaaa1111",
        "references_file",
        "file",
        "demo:src/queue.rs",
    );

    assert_eq!(edge_fts::refresh(&mut conn).expect("first"), 2);
    let first = dump(&conn);
    assert_eq!(edge_fts::refresh(&mut conn).expect("second"), 2);

    // Same rowids, same text: refresh is a pure function of `edges`.
    assert_eq!(dump(&conn), first);
}

#[test]
fn permuted_edge_insert_order_indexes_identically() {
    let seeds: [(&str, &str, &str); 3] = [
        ("aaaa1111", "supersedes", "bbbb2222"),
        ("bbbb2222", "derived_from", "cccc3333"),
        ("aaaa1111", "conflicts_with", "cccc3333"),
    ];
    let mut dumps = Vec::new();
    for order in [[0usize, 1, 2], [2, 0, 1]] {
        let (_dir, mut conn) = open_db();
        seed_memory(&conn, "aaaa1111", "decision", "queue-design-v3");
        seed_memory(&conn, "bbbb2222", "note", "queue-design-v2");
        seed_memory(&conn, "cccc3333", "note", "queue-design-v1");
        for i in order {
            let (src, rel, dst) = seeds[i];
            seed_relation(&conn, src, rel, dst);
        }
        edge_fts::refresh(&mut conn).expect("refresh");
        dumps.push((dump(&conn), keys(&conn, "queue design")));
    }
    assert_eq!(dumps[0].0, dumps[1].0, "rowids/text differ by insert order");
    assert_eq!(
        dumps[0].1, dumps[1].1,
        "query results differ by insert order"
    );
}

#[test]
fn refresh_drops_rows_for_edges_that_no_longer_exist() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_memory(&conn, "bbbb2222", "note", "queue-design-v1");
    seed_relation(&conn, "aaaa1111", "supersedes", "bbbb2222");
    edge_fts::refresh(&mut conn).expect("refresh");
    assert_eq!(hits(&conn, "queue design").len(), 1);

    conn.execute("DELETE FROM edges", []).expect("purge edges");
    assert_eq!(edge_fts::refresh(&mut conn).expect("refresh"), 0);

    assert!(hits(&conn, "queue design").is_empty(), "stale row survived");
}

#[test]
fn needs_refresh_is_true_only_for_an_unindexed_non_empty_graph() {
    let (_dir, mut conn) = open_db();
    // Empty graph: nothing to heal.
    assert!(!edge_fts::needs_refresh(&conn).expect("empty"));

    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_memory(&conn, "bbbb2222", "note", "queue-design-v1");
    seed_relation(&conn, "aaaa1111", "supersedes", "bbbb2222");
    // The upgrade state the 0012 migration leaves behind: edges, no index.
    assert!(edge_fts::needs_refresh(&conn).expect("unindexed"));

    edge_fts::refresh(&mut conn).expect("refresh");
    assert!(!edge_fts::needs_refresh(&conn).expect("indexed"));
}
