//! Pagination and ranking-parity tests for
//! [`comemory::retrieval::pipeline`] — split from `retrieval__pipeline.rs`
//! to keep each test binary under the size cap.
//!
//! Covers `pool_size` / `paginate` unit behavior, the end-to-end STABILITY
//! property of deep paging (paging deeper — a larger candidate pool — must
//! not reorder or drop earlier pages, because RRF rank-fusion and MMR/
//! near-dup selection keep a stable top prefix as the pool grows), and the
//! committed baseline ranking snapshot over the shared smoke corpus.

use assert_cmd::Command;
use comemory::retrieval::pipeline::{PageWindow, SearchOptions, paginate, pool_size, search};
use comemory::retrieval::router::CANDIDATE_POOL;
use comemory::simhash::{NEAR_DUP_HAMMING, hamming64};

// Included via `#[path]` rather than a declaration in `tests/common/mod.rs`
// so only the binaries that actually call the corpus pull it in — same
// pattern as `tests/cli_rank_smoke.rs`.
#[path = "common/corpus.rs"]
mod corpus;

// ── pool_size unit tests ────────────────────────────────────────────────

#[test]
fn pool_size_floors_at_candidate_pool() {
    // A small first page still fetches at least CANDIDATE_POOL so the
    // ranking pool is never starved.
    assert_eq!(pool_size(0, 5, 200), CANDIDATE_POOL);
    assert_eq!(pool_size(0, 12, 200), CANDIDATE_POOL);
}

#[test]
fn pool_size_adds_one_page_buffer_for_deeper_pages() {
    // offset + limit + buffer(=limit): page 2 of size 30 wants
    // 30 + 30 + 30 = 90 candidates, above the CANDIDATE_POOL floor.
    assert_eq!(pool_size(30, 30, 200), 90);
}

#[test]
fn pool_size_clamps_to_max_window() {
    // The window ceiling caps the pool no matter how deep the request.
    assert_eq!(pool_size(180, 40, 200), 200);
    assert_eq!(pool_size(500, 50, 200), 200);
}

#[test]
fn pool_size_limit_zero_fetches_whole_window() {
    // "all within the window" pulls the entire max_window.
    assert_eq!(pool_size(0, 0, 200), 200);
    assert_eq!(pool_size(40, 0, 200), 200);
}

// ── paginate unit tests ─────────────────────────────────────────────────

#[test]
fn paginate_slices_each_page_without_overlap_or_gap() {
    let ranked: Vec<u32> = (0..25).collect();
    // Page 0
    let (p0, more0, total0) = paginate(
        ranked.clone(),
        PageWindow {
            offset: 0,
            limit: 10,
        },
        200,
    );
    assert_eq!(p0, (0..10).collect::<Vec<_>>());
    assert!(more0);
    assert_eq!(total0, 25);
    // Page 1
    let (p1, more1, _) = paginate(
        ranked.clone(),
        PageWindow {
            offset: 10,
            limit: 10,
        },
        200,
    );
    assert_eq!(p1, (10..20).collect::<Vec<_>>());
    assert!(more1);
    // Page 2 (last)
    let (p2, more2, _) = paginate(
        ranked.clone(),
        PageWindow {
            offset: 20,
            limit: 10,
        },
        200,
    );
    assert_eq!(p2, (20..25).collect::<Vec<_>>());
    assert!(!more2, "last page must have has_more=false");
    // Concatenated pages reproduce the full list exactly (no overlap/gap).
    let mut joined = p0;
    joined.extend(p1);
    joined.extend(p2);
    assert_eq!(joined, ranked);
}

#[test]
fn paginate_offset_past_end_is_empty() {
    let ranked: Vec<u32> = (0..5).collect();
    let (page, more, total) = paginate(
        ranked,
        PageWindow {
            offset: 99,
            limit: 10,
        },
        200,
    );
    assert!(page.is_empty(), "offset past end yields empty page");
    assert!(!more);
    assert_eq!(total, 5);
}

#[test]
fn paginate_has_more_false_at_window_ceiling() {
    // 50 ranked results but max_window is 20: a page ending exactly at the
    // ceiling must report has_more=false even though more ranked rows exist
    // beyond it — deeper results require refining the query.
    let ranked: Vec<u32> = (0..50).collect();
    let (page, more, _total) = paginate(
        ranked,
        PageWindow {
            offset: 10,
            limit: 10,
        },
        20,
    );
    assert_eq!(page, (10..20).collect::<Vec<_>>());
    assert!(
        !more,
        "offset+limit == max_window must force has_more=false"
    );
}

#[test]
fn paginate_limit_zero_returns_remaining_with_no_more() {
    let ranked: Vec<u32> = (0..30).collect();
    let (page, more, total) = paginate(
        ranked,
        PageWindow {
            offset: 5,
            limit: 0,
        },
        200,
    );
    assert_eq!(page, (5..30).collect::<Vec<_>>());
    assert!(!more, "limit==0 returns everything remaining, so no more");
    assert_eq!(total, 30);
}

/// Seed `n` lexically-matching memories with well-spread SimHashes (no
/// near-dup collapse) and varied token counts so the rerank produces a
/// strict ranked order. Returns the open connection.
fn seed_paging_corpus(n: u64) -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    let mut sims: Vec<u64> = Vec::new();
    for i in 0..n {
        let sim = (i + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for prev in &sims {
            assert!(
                hamming64(*prev, sim) > NEAR_DUP_HAMMING,
                "fixture simhashes must not collapse as near-dups"
            );
        }
        sims.push(sim);
        let id = format!("c{i:07}");
        // Distinct token counts give distinct BM25 → a strict ranked order.
        let pad = "sqlite ".repeat((i % 5 + 1) as usize);
        let body = format!("sqlite paging topic {i} {pad}");
        conn.execute(
            "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                                  body, created_at, updated_at, md_path, simhash)
             VALUES (?1, ?2, 'note', 'd', 'f', 3, 1, ?3, ?4,
                     '2026-06-09T00:00:00Z', '2026-06-09T00:00:00Z', ?5, ?6)",
            rusqlite::params![
                id,
                format!("s{i}"),
                format!("h{i}"),
                body,
                format!("m/{i}.md"),
                sim as i64
            ],
        )
        .expect("seed memory");
        conn.execute(
            "INSERT INTO memory_fts(memory_id, body, tags) VALUES (?1, ?2, '')",
            rusqlite::params![id, body],
        )
        .expect("seed fts");
    }
    (dir, conn)
}

fn page_ids(
    cfg: &comemory::config::Config,
    conn: &rusqlite::Connection,
    offset: usize,
    limit: usize,
) -> (Vec<String>, bool, usize) {
    let run = search(
        cfg,
        conn,
        "sqlite",
        None,
        None,
        None,
        SearchOptions {
            track: false,
            source: "search",
            window: PageWindow { offset, limit },
        },
    )
    .expect("search");
    let ids = run.hits.iter().map(|h| h.memory_id.clone()).collect();
    (ids, run.has_more, run.total)
}

/// THE stability test: paging through the full result set with a fixed
/// page size and increasing offset must (a) never repeat an id across
/// pages, (b) never skip an in-window id, and (c) reproduce — in order —
/// the single-shot ranked window. This empirically proves the bounded
/// ranked window is stable under RRF + MMR/near-dup as the pool grows.
#[test]
fn deep_paging_is_stable_no_overlap_no_gap_consistent_order() {
    let (_d, conn) = seed_paging_corpus(40);
    let cfg = comemory::config::Config::defaults();

    // The single large-`k` window over the same corpus: page size = whole
    // window (limit 0), offset 0. This is the ground-truth ordering.
    let (full, full_more, full_total) = page_ids(&cfg, &conn, 0, 0);
    assert!(
        !full_more,
        "the whole-window page can have nothing beyond it"
    );
    assert_eq!(
        full.len(),
        full_total,
        "total must equal the in-window count"
    );
    assert_eq!(full.len(), 40, "all 40 lexical matches fit in the window");

    // Page through with a fixed page size of 7.
    let page_size = 7;
    let mut seen = std::collections::HashSet::new();
    let mut concatenated: Vec<String> = Vec::new();
    let mut offset = 0;
    loop {
        let (ids, has_more, total) = page_ids(&cfg, &conn, offset, page_size);
        assert_eq!(
            total, 40,
            "total is the in-window ranked count on every page"
        );
        for id in &ids {
            assert!(
                seen.insert(id.clone()),
                "id {id} appeared on two pages (overlap)"
            );
        }
        concatenated.extend(ids.iter().cloned());
        // has_more must be exactly "there is a next page".
        let expect_more = offset + page_size < full.len();
        assert_eq!(has_more, expect_more, "has_more wrong at offset {offset}");
        if !has_more {
            break;
        }
        offset += page_size;
    }
    // (b) no in-window id skipped + (c) ordering identical to single-shot.
    assert_eq!(
        concatenated, full,
        "concatenated pages must reproduce the single-shot ranked window in order"
    );
}

#[test]
fn pages_match_single_shot_prefix_at_every_depth() {
    // A page [offset, offset+k] must equal the single-shot ranked window
    // sliced the same way — the head never reorders as we page deeper.
    let (_d, conn) = seed_paging_corpus(40);
    let cfg = comemory::config::Config::defaults();
    let (full, _, _) = page_ids(&cfg, &conn, 0, 0);
    for offset in [0usize, 5, 12, 25, 33] {
        let k = 7;
        let (page, _, _) = page_ids(&cfg, &conn, offset, k);
        let end = (offset + k).min(full.len());
        let start = offset.min(full.len());
        assert_eq!(
            page,
            full[start..end].to_vec(),
            "page at offset {offset} must equal the single-shot slice"
        );
    }
}

#[test]
fn offset_beyond_window_yields_empty_page_and_no_more() {
    let (_d, conn) = seed_paging_corpus(15);
    let cfg = comemory::config::Config::defaults();
    let (ids, has_more, total) = page_ids(&cfg, &conn, 9999, 10);
    assert!(ids.is_empty(), "offset past the window must be empty");
    assert!(!has_more, "nothing beyond an out-of-range offset");
    assert_eq!(total, 15, "total still reports the in-window ranked count");
}

// ── baseline ranking parity snapshot ────────────────────────────────────

/// Seed a temp data dir with the shared smoke corpus through the real
/// `comemory save` binary — markdown, FTS5 rows (identifier tokenizer) and
/// SimHashes are written exactly as production writes them — then open the
/// resulting `comemory.db` for in-process pipeline calls.
fn seed_smoke_corpus() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_dir = dir.path().join(".comemory");
    for (kind, body, tags, quality) in corpus::CORPUS {
        let quality = quality.to_string();
        Command::cargo_bin("comemory")
            .expect("cargo_bin comemory")
            .env("COMEMORY_DATA_DIR", &data_dir)
            .args(["save", body, "--kind", kind, "--tags", tags, "--quality"])
            .arg(&quality)
            .assert()
            .success();
    }
    let conn = comemory::store::connection::open(data_dir.join("comemory.db")).expect("open");
    (dir, conn)
}

/// Committed baseline: the ordered result ids the pipeline returns for
/// every `SMOKE_QUERIES` entry, lexical-only (no vector) with tracking off
/// so nothing the run observes is mutated by the run itself.
///
/// Ids are content-derived (8-hex of the body) and every scoring input is
/// run-invariant here — access counts stay 0, no feedback exists, and all
/// 20 memories are saved on the same day so ACT-R recency is uniform — so
/// the snapshot is stable across repeated runs and machines.
///
/// This is a *parity* fixture, not a quality bar: it pins today's ranking
/// so a later ranking change can prove its disabled path is bit-for-bit
/// the legacy one. Body edits to the corpus legitimately churn it (ids are
/// derived from bodies); an unexplained diff on unchanged corpus text is
/// the regression this catches.
#[test]
fn baseline_smoke_queries_ids_order() {
    let (_d, conn) = seed_smoke_corpus();
    let cfg = comemory::config::Config::defaults();
    let rendered = render_smoke_ranking(&cfg, &conn);
    insta::assert_snapshot!(rendered);
}

/// Run every `SMOKE_QUERIES` entry through the pipeline under `cfg` and
/// render `query` / indented result ids — the ids-and-order projection both
/// parity assertions compare.
fn render_smoke_ranking(cfg: &comemory::config::Config, conn: &rusqlite::Connection) -> String {
    let mut rendered = String::new();
    for (query, _expected) in corpus::SMOKE_QUERIES {
        let run = search(
            cfg,
            conn,
            query,
            None,
            None,
            None,
            SearchOptions {
                track: false,
                source: "search",
                window: PageWindow::top_k(cfg),
            },
        )
        .expect("search");
        // Guard against an all-empty snapshot silently passing forever.
        assert!(!run.hits.is_empty(), "query {query:?} returned no hits");
        rendered.push_str(query);
        rendered.push('\n');
        for hit in &run.hits {
            rendered.push_str("  ");
            rendered.push_str(&hit.memory_id);
            rendered.push('\n');
        }
    }
    rendered
}

/// Count the `edges` rows the graph-expansion leg is allowed to traverse.
fn walkable_edges(conn: &rusqlite::Connection) -> i64 {
    let rels = comemory::retrieval::graph_route::ALLOWED_RELS
        .iter()
        .map(|rel| format!("'{rel}'"))
        .collect::<Vec<_>>()
        .join(",");
    conn.query_row(
        &format!("SELECT count(*) FROM edges WHERE rel IN ({rels})"),
        [],
        |r| r.get(0),
    )
    .expect("count edges")
}

/// AC-4 parity: `graph_hops = 0` short-circuits to the legacy code path, so
/// it must still reproduce the ranking committed *before* the graph leg
/// existed — this asserts against that same pre-change snapshot, which is
/// ground truth and must never be regenerated to make this pass.
///
/// The default-config run is held to it too, with the reason pinned rather
/// than assumed: `comemory save` writes only the hub edges `in_repo`,
/// `authored_by` and `tagged` for this corpus (no `--supersedes`, no
/// backtick-fenced code references in any body), and every hub rel is
/// outside `ALLOWED_RELS` — so the expansion is empty and the leg's second
/// short-circuit returns the provisional ranking untouched.
#[test]
fn graph_hops_zero_reproduces_the_pre_change_ranking() {
    let (_d, conn) = seed_smoke_corpus();
    let mut cfg = comemory::config::Config::defaults();
    assert_eq!(
        cfg.retrieval.graph_hops, 2,
        "the default config must enable the leg, or this proves nothing"
    );
    let with_leg = render_smoke_ranking(&cfg, &conn);

    cfg.retrieval.graph_hops = 0;
    let rendered = render_smoke_ranking(&cfg, &conn);
    insta::assert_snapshot!("baseline_smoke_queries_ids_order", rendered);

    assert_eq!(
        walkable_edges(&conn),
        0,
        "the smoke corpus carries no expansion-eligible edge"
    );
    assert_eq!(
        with_leg, rendered,
        "an empty expansion must leave the ranking untouched"
    );
}
