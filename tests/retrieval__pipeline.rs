//! Tests for [`comemory::retrieval::pipeline::search`] — the end-to-end
//! route → rerank → diversify → top-k path plus best-effort access
//! tracking and query logging.

use comemory::config::Config;
use comemory::retrieval::pipeline::{PageWindow, SearchOptions, search};
use comemory::retrieval::rerank::Reranked;
use comemory::retrieval::router::Source;
use comemory::retrieval::scope::{Filters, TimeScope};
use comemory::retrieval::score::SUPERSEDE_PENALTY;
use comemory::simhash::{NEAR_DUP_HAMMING, hamming64};

/// SimHash for the `nth` fixture memory, spread by a golden-ratio multiply
/// so no two fixtures land within `NEAR_DUP_HAMMING` of each other and get
/// collapsed by the diversify stage (the spread is asserted for this whole
/// family in `pipeline_cuts_to_configured_top_k`).
fn spread_simhash(nth: u64) -> u64 {
    (nth + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// Open a freshly migrated `comemory.db` inside a tempdir.
fn open_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    (dir, conn)
}

/// Insert one live `memories` row plus the matching `memory_fts` row.
fn seed_body(conn: &rusqlite::Connection, id: &str, body: &str, nth: u64) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash)
         VALUES (?1, ?1, 'note', 'd', 'f', 3, 1, ?1, ?2,
                 '2026-06-09T00:00:00Z', '2026-06-09T00:00:00Z', ?1, ?3)",
        rusqlite::params![id, body, spread_simhash(nth) as i64],
    )
    .expect("seed memory");
    conn.execute(
        "INSERT INTO memory_fts(memory_id, body, tags) VALUES (?1, ?2, '')",
        rusqlite::params![id, body],
    )
    .expect("seed fts");
}

/// Insert one memory→memory `edges` row in the production `(kind, id)`
/// addressing, oriented as `store::memory_row` writes frontmatter relations:
/// `src` is the memory that declares the relation, so a `supersedes` row
/// reads "`src` replaces `dst`".
fn relate(conn: &rusqlite::Connection, src: &str, rel: &str, dst: &str) {
    conn.execute(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory', ?1, 'memory', ?2, ?3, '2026-06-09T00:00:00Z')",
        rusqlite::params![src, dst, rel],
    )
    .expect("seed edge");
}

fn seeded() -> (tempfile::TempDir, rusqlite::Connection) {
    let (dir, conn) = open_db();
    seed_body(&conn, "aaaa0001", "sqlite busy timeout fix for pool", 0);
    (dir, conn)
}

/// One lexical-only search with tracking off, returning just the hits.
fn run_search(cfg: &Config, conn: &rusqlite::Connection, query: &str) -> Vec<Reranked> {
    search(
        cfg,
        conn,
        query,
        None,
        Filters::none(),
        SearchOptions {
            track: false,
            source: "search",
            window: PageWindow::top_k(cfg),
        },
    )
    .expect("search")
    .hits
}

/// Result ids in returned order.
fn ids(hits: &[Reranked]) -> Vec<&str> {
    hits.iter().map(|h| h.memory_id.as_str()).collect()
}

/// The hit for `id`, or a failure naming what actually came back.
fn pick<'a>(hits: &'a [Reranked], id: &str) -> &'a Reranked {
    hits.iter()
        .find(|h| h.memory_id == id)
        .unwrap_or_else(|| panic!("{id} missing from results {:?}", ids(hits)))
}

#[test]
fn search_returns_reranked_diversified_hits() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let run = search(
        &cfg,
        &conn,
        "sqlite busy",
        None,
        Filters::none(),
        SearchOptions {
            track: false,
            source: "search",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search");
    assert_eq!(run.hits.len(), 1);
    assert_eq!(run.hits[0].memory_id, "aaaa0001");
    assert!(run.hits[0].parts.final_score > 0.0);
}

#[test]
fn retrieval_hit_bumps_access_tracking() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    search(
        &cfg,
        &conn,
        "sqlite busy",
        None,
        Filters::none(),
        SearchOptions {
            track: true,
            source: "search",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search");
    let (count, last): (i64, String) = conn
        .query_row(
            "SELECT access_count, last_accessed FROM memories WHERE id='aaaa0001'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(count, 1);
    assert!(
        last.as_str() > "2026-06-09T00:00:00Z",
        "last_accessed updated, got {last}"
    );
}

#[test]
fn access_tracking_failure_does_not_break_reads() {
    let (_d, conn) = seeded();
    // Make every write fail: query_only rejects the access-tracking UPDATE
    // and the retrieval_log INSERT while leaving the read path untouched.
    conn.pragma_update(None, "query_only", true)
        .expect("pragma");
    let cfg = comemory::config::Config::defaults();
    let run = search(
        &cfg,
        &conn,
        "sqlite busy",
        None,
        Filters::none(),
        SearchOptions {
            track: true,
            source: "search",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search must succeed when access tracking cannot write");
    assert_eq!(run.hits.len(), 1);
    assert_eq!(run.hits[0].memory_id, "aaaa0001");
    // Query logging is best-effort too: a failed INSERT yields no id.
    assert!(run.query_id.is_none(), "logging failed, id must be None");
    // The bump itself was skipped, not silently rerouted somewhere else.
    let count: i64 = conn
        .query_row(
            "SELECT access_count FROM memories WHERE id='aaaa0001'",
            [],
            |r| r.get(0),
        )
        .expect("row");
    assert_eq!(count, 0);
}

#[test]
fn search_with_track_logs_one_retrieval_log_row() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let run = search(
        &cfg,
        &conn,
        "sqlite busy",
        None,
        Filters::none(),
        SearchOptions {
            track: true,
            source: "search",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search");
    let qid = run.query_id.expect("query_id present when tracking");
    assert!(comemory::stats::feedback::is_valid_query_id(&qid));
    let (q, ids, dur): (String, String, Option<i64>) = conn
        .query_row(
            "SELECT query, returned_ids, duration_ms FROM retrieval_log WHERE query_id = ?1",
            [&qid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("logged");
    assert_eq!(q, "sqlite busy");
    let parsed: Vec<String> = serde_json::from_str(&ids).expect("json ids");
    assert_eq!(parsed.len(), run.hits.len());
    assert_eq!(parsed[0], "aaaa0001");
    assert!(dur.is_some());
}

#[test]
fn search_with_filters_logs_repo_kind_and_source() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let run = search(
        &cfg,
        &conn,
        "sqlite busy",
        None,
        Filters {
            repo: Some("d"),
            kind: Some("note"),
            ..Filters::none()
        },
        SearchOptions {
            track: true,
            source: "search",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search");
    let qid = run.query_id.expect("query_id present when tracking");
    let (repo, kind, source): (Option<String>, Option<String>, String) = conn
        .query_row(
            "SELECT repo, kind, source FROM retrieval_log WHERE query_id = ?1",
            [&qid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("logged");
    assert_eq!(repo.as_deref(), Some("d"), "repo filter logged verbatim");
    assert_eq!(kind.as_deref(), Some("note"), "kind filter logged verbatim");
    assert_eq!(source, "search");
}

#[test]
fn search_without_filters_logs_null_repo_and_kind() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let run = search(
        &cfg,
        &conn,
        "sqlite busy",
        None,
        Filters::none(),
        SearchOptions {
            track: true,
            source: "context",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search");
    let qid = run.query_id.expect("query_id present when tracking");
    let (repo, kind, source): (Option<String>, Option<String>, String) = conn
        .query_row(
            "SELECT repo, kind, source FROM retrieval_log WHERE query_id = ?1",
            [&qid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("logged");
    assert_eq!(repo, None, "no repo filter must log NULL");
    assert_eq!(kind, None, "no kind filter must log NULL");
    assert_eq!(source, "context", "caller-declared source logged verbatim");
}

#[test]
fn search_without_track_logs_nothing_and_freezes_access() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let before: i64 = conn
        .query_row(
            "SELECT access_count FROM memories WHERE id='aaaa0001'",
            [],
            |r| r.get(0),
        )
        .expect("row");
    for _ in 0..2 {
        let run = search(
            &cfg,
            &conn,
            "sqlite busy",
            None,
            Filters::none(),
            SearchOptions {
                track: false,
                source: "search",
                window: PageWindow::top_k(&cfg),
            },
        )
        .expect("search");
        assert!(run.query_id.is_none(), "no query_id when track is off");
        assert_eq!(run.hits.len(), 1);
    }
    let logged: i64 = conn
        .query_row("SELECT count(*) FROM retrieval_log", [], |r| r.get(0))
        .expect("count");
    assert_eq!(logged, 0, "track:false must not write retrieval_log");
    let after: i64 = conn
        .query_row(
            "SELECT access_count FROM memories WHERE id='aaaa0001'",
            [],
            |r| r.get(0),
        )
        .expect("row");
    assert_eq!(after, before, "track:false must not bump access_count");
}

#[test]
fn pipeline_cuts_to_configured_top_k() {
    let (_d, conn) = open_db();
    // 15 distinct memories matching the single term "sqlite". The loop
    // asserts pairwise Hamming > NEAR_DUP_HAMMING over the whole
    // `spread_simhash` family, keeping every fixture in this file honest
    // about not collapsing as a near-dup.
    let mut sims: Vec<u64> = Vec::new();
    for i in 0..15u64 {
        let sim = spread_simhash(i);
        for prev in &sims {
            assert!(
                hamming64(*prev, sim) > NEAR_DUP_HAMMING,
                "fixture simhashes must not collapse as near-dups"
            );
        }
        sims.push(sim);
        seed_body(
            &conn,
            &format!("bbbb{i:04}"),
            &format!("sqlite topic number {i}"),
            i,
        );
    }
    let cfg = Config::defaults();
    assert_eq!(
        cfg.retrieval.top_k, 12,
        "default top_k expected by this test"
    );
    let run = search(
        &cfg,
        &conn,
        "sqlite",
        None,
        Filters::none(),
        SearchOptions {
            track: false,
            source: "search",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search");
    assert_eq!(run.hits.len(), 12, "pipeline must cut to top_k");
}

// ── graph-expansion leg, end to end ─────────────────────────────────────

/// `"sqlite pool"` matches X (`aaaa0001`) and Y (`bbbb0002`); X's body is
/// the shorter of the two, so BM25 puts it at provisional rank 1 and Y at
/// rank 2. The dark memory D (`dddd0004`) shares no token with the query and
/// hangs off Y alone, so it is reachable only when Y is allowed to seed the
/// walk.
fn seed_seed_truncation_corpus() -> (tempfile::TempDir, rusqlite::Connection) {
    let (dir, conn) = open_db();
    seed_body(&conn, "aaaa0001", "sqlite pool", 1);
    seed_body(
        &conn,
        "bbbb0002",
        "sqlite pool timeout retry backoff jitter ceiling window",
        2,
    );
    seed_body(&conn, "dddd0004", "sourdough starter hydration ratio", 3);
    relate(&conn, "bbbb0002", "derived_from", "dddd0004");
    (dir, conn)
}

/// A lexical seed A (`aaaa0001`) with two dark one-hop neighbors: D
/// (`dddd0004`), replaced by the live memory S (`ssss0006`), and its
/// un-superseded twin E (`eeee0005`). D's id sorts before E's, so the leg's
/// `(hops, id)` order ranks D higher than E — any inversion in the final
/// output is the supersede penalty, not relevance. S sits two hops from A,
/// so a one-hop walk leaves it out of the candidate pool entirely.
fn seed_superseded_neighbor_corpus() -> (tempfile::TempDir, rusqlite::Connection) {
    let (dir, conn) = open_db();
    seed_body(&conn, "aaaa0001", "sqlite pool", 1);
    seed_body(&conn, "dddd0004", "sourdough starter hydration ratio", 2);
    seed_body(&conn, "eeee0005", "kiln glaze firing schedule chart", 3);
    seed_body(&conn, "ssss0006", "harbour ferry timetable revision", 4);
    relate(&conn, "aaaa0001", "derived_from", "dddd0004");
    relate(&conn, "aaaa0001", "derived_from", "eeee0005");
    relate(&conn, "ssss0006", "supersedes", "dddd0004");
    (dir, conn)
}

/// AC-10: `graph_seeds` truncates the seed set at the caller, and that cut
/// is observable through the public `pipeline::search` surface — a dark
/// neighbor of the provisional *second* hit only exists in the results once
/// the second hit is allowed to seed the walk.
#[test]
fn graph_seeds_bounds_which_provisional_hits_expand() {
    let (_d, conn) = seed_seed_truncation_corpus();
    let mut cfg = Config::defaults();
    cfg.retrieval.graph_hops = 1;

    cfg.retrieval.graph_seeds = 1;
    let one = run_search(&cfg, &conn, "sqlite pool");
    assert_eq!(
        ids(&one),
        ["aaaa0001", "bbbb0002"],
        "X is rank 1 and Y rank 2; with one seed only X expands, so D stays out"
    );

    cfg.retrieval.graph_seeds = 2;
    let two = run_search(&cfg, &conn, "sqlite pool");
    let dark = pick(&two, "dddd0004");
    assert_eq!(
        dark.source,
        Source::Graph,
        "D has no lexical match — the graph leg is the only way in"
    );
    assert_eq!(dark.tier, 0, "graph candidates never walked the ladder");
}

/// AC-11: being superseded does not exempt a memory from the graph leg, and
/// does not double-charge it either — it surfaces, and carries exactly the
/// one supersede factor rerank applies to every other candidate.
#[test]
fn superseded_dark_neighbor_surfaces_and_keeps_the_supersede_penalty() {
    let (_d, conn) = seed_superseded_neighbor_corpus();
    let mut cfg = Config::defaults();
    cfg.retrieval.graph_hops = 1;
    let hits = run_search(&cfg, &conn, "sqlite pool");

    let dark = pick(&hits, "dddd0004");
    let twin = pick(&hits, "eeee0005");
    assert_eq!(dark.source, Source::Graph, "the leg still surfaces it");
    assert_eq!(dark.superseded_by.as_deref(), Some("ssss0006"));
    assert_eq!(twin.superseded_by, None, "the twin is nobody's predecessor");

    assert!(
        (dark.parts.supersede - SUPERSEDE_PENALTY).abs() < 1e-12,
        "exactly rerank's penalty, applied once: {:?}",
        dark.parts
    );
    assert!((twin.parts.supersede - 1.0).abs() < 1e-12);
    let expected = f64::from(dark.parts.rrf)
        * dark.parts.activation
        * dark.parts.feedback
        * dark.parts.quality
        * dark.parts.supersede;
    assert!(
        (dark.parts.final_score - expected).abs() < 1e-6,
        "final_score must be the published product of its parts: {:?}",
        dark.parts
    );

    assert!(
        dark.parts.rrf >= twin.parts.rrf,
        "the leg ranks D at least as high as its twin: {:?} vs {:?}",
        dark.parts,
        twin.parts
    );
    assert!(
        dark.parts.final_score < twin.parts.final_score,
        "so the demotion below the twin is the penalty alone: {:?} vs {:?}",
        dark.parts,
        twin.parts
    );
}

/// AC-7: with tracking off, a run mutates nothing it also reads, so two
/// identical searches over an edge-expanded corpus must return the same ids
/// in the same order — including the leg's `(hops, id)` tie-break across
/// two different depths.
#[test]
fn repeated_searches_over_graph_edges_return_identical_order() {
    let (_d, conn) = seed_superseded_neighbor_corpus();
    let mut cfg = Config::defaults();
    cfg.retrieval.graph_hops = 2;

    let first = run_search(&cfg, &conn, "sqlite pool");
    assert!(
        first.iter().any(|h| h.source == Source::Graph),
        "the graph leg must have contributed, else determinism is untested: {:?}",
        ids(&first)
    );
    let second = run_search(&cfg, &conn, "sqlite pool");
    assert_eq!(
        ids(&first),
        ids(&second),
        "repeated identical searches must not reorder"
    );
}

/// Run one lexical-only search under `scope`, tracking off, returning ids.
fn scoped_ids(cfg: &Config, conn: &rusqlite::Connection, scope: &TimeScope) -> Vec<String> {
    search(
        cfg,
        conn,
        "sqlite pool",
        None,
        Filters {
            scope,
            ..Filters::none()
        },
        SearchOptions {
            track: false,
            source: "search",
            window: PageWindow::top_k(cfg),
        },
    )
    .expect("search")
    .hits
    .into_iter()
    .map(|h| h.memory_id)
    .collect()
}

#[test]
fn scope_cutoff_excludes_a_candidate_end_to_end() {
    // The scope has to survive every stage — route, rerank, diversify and
    // the page cut — so this asserts on what `search` actually returns,
    // not on the candidate pool.
    let (_d, conn) = open_db();
    seed_body(&conn, "aaaa0001", "sqlite pool tuning notes", 0);
    seed_body(&conn, "bbbb0002", "sqlite pool sizing rewrite", 1);
    conn.execute(
        "UPDATE memories SET created_at = '2026-08-01T00:00:00Z' WHERE id = 'bbbb0002'",
        [],
    )
    .expect("re-date the later memory");
    let cfg = Config::defaults();

    let unscoped = scoped_ids(&cfg, &conn, &TimeScope::none());
    assert_eq!(unscoped.len(), 2, "baseline sees both: {unscoped:?}");

    let scoped = scoped_ids(
        &cfg,
        &conn,
        &TimeScope {
            cutoff: Some("2026-07-01T00:00:00Z".to_string()),
            ..TimeScope::none()
        },
    );
    assert_eq!(
        scoped,
        ["aaaa0001"],
        "the memory created after the cutoff must not reach the caller"
    );
}
