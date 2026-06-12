//! Tests for [`comemory::retrieval::code_rerank`].
//!
//! Seeds real `code_symbols` / `code_feedback` / `edges` rows through the
//! production writers (`code_row::insert` + direct updates) and, for the
//! working-set affinity case, a real on-disk git repository. Asserts that
//! the four bounded priors (PageRank, activation, affinity, feedback)
//! reorder candidates deterministically and that cAST chunk rows coalesce
//! onto their parent identity.

#[path = "../common/git_setup.rs"]
mod git_setup;

use comemory::config::Config;
use comemory::retrieval::code_rerank::{rerank_code, working_set, CodeReranked, WorkingSet};
use comemory::retrieval::code_route::CodeRoutedHit;
use comemory::retrieval::router::Source;
use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::connection;

fn open_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// Insert one real `code_symbols` row via the production writer and
/// return its rowid.
fn seed(
    conn: &rusqlite::Connection,
    repo: &str,
    path: &str,
    symbol: &str,
    lines: (i64, i64),
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
            line_start: lines.0,
            line_end: lines.1,
            snippet: "fn body() {}",
            simhash: 0,
            parent_id,
        },
    )
    .expect("insert code symbol")
}

fn hit(symbol_id: i64, score: f32) -> CodeRoutedHit {
    CodeRoutedHit {
        symbol_id,
        score,
        source: Source::Lexical,
    }
}

fn by_id(out: &[CodeReranked], id: i64) -> &CodeReranked {
    out.iter()
        .find(|r| r.symbol_id == id)
        .expect("symbol present in output")
}

#[test]
fn higher_rank_score_wins_equal_relevance() {
    let (_d, conn) = open_db();
    let cfg = Config::defaults();
    let central = seed(&conn, "demo", "core.rs", "core::run", (1, 10), None);
    let leaf = seed(&conn, "demo", "leaf.rs", "leaf::run", (1, 10), None);
    conn.execute(
        "UPDATE code_symbols SET rank_score = 0.9 WHERE id = ?1",
        [central],
    )
    .expect("bump rank");
    conn.execute(
        "UPDATE code_symbols SET rank_score = 0.1 WHERE id = ?1",
        [leaf],
    )
    .expect("set low rank");

    let out = rerank_code(
        &conn,
        &cfg,
        &[hit(central, 1.0), hit(leaf, 1.0)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out[0].symbol_id, central);
    assert!(
        by_id(&out, central).parts.rank > by_id(&out, leaf).parts.rank,
        "central file must carry the larger rank boost"
    );
}

#[test]
fn unranked_repo_keeps_rank_prior_neutral() {
    // Both rank_scores stay at the 0.0 column default → pool median 0 →
    // every rank prior must be exactly 1.0 (no division blow-up).
    let (_d, conn) = open_db();
    let cfg = Config::defaults();
    let a = seed(&conn, "demo", "a.rs", "a::run", (1, 10), None);
    let b = seed(&conn, "demo", "b.rs", "b::run", (1, 10), None);
    let out = rerank_code(
        &conn,
        &cfg,
        &[hit(a, 1.0), hit(b, 1.0)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    for r in &out {
        assert!((r.parts.rank - 1.0).abs() < 1e-12, "got {}", r.parts.rank);
    }
}

#[test]
fn access_count_bump_flips_near_tie() {
    let (_d, conn) = open_db();
    let cfg = Config::defaults();
    let a = seed(&conn, "demo", "a.rs", "a::run", (1, 10), None);
    let b = seed(&conn, "demo", "b.rs", "b::run", (1, 10), None);

    // Baseline: identical priors → tie breaks on ascending symbol_id.
    let out = rerank_code(
        &conn,
        &cfg,
        &[hit(a, 1.0), hit(b, 1.0)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out[0].symbol_id, a, "tie must break on ascending id");

    // Bump the would-be loser's access count: activation must flip the tie.
    conn.execute(
        "UPDATE code_symbols SET access_count = 50 WHERE id = ?1",
        [b],
    )
    .expect("bump access");
    let out = rerank_code(
        &conn,
        &cfg,
        &[hit(a, 1.0), hit(b, 1.0)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out[0].symbol_id, b, "accessed symbol must outrank its twin");
    assert!(by_id(&out, b).parts.activation > 1.0);
}

#[test]
fn irrelevant_heavy_symbol_sinks_below_neutral_twin() {
    let (_d, conn) = open_db();
    let cfg = Config::defaults();
    let a = seed(&conn, "demo", "a.rs", "a::run", (1, 10), None);
    let b = seed(&conn, "demo", "b.rs", "b::run", (1, 10), None);
    // `a` would win the symbol_id tie-break; downvotes must sink it below b.
    conn.execute(
        "INSERT INTO code_feedback(repo, path, symbol, used_count, irrelevant_count) \
         VALUES ('demo', 'a.rs', 'a::run', 0, 20)",
        [],
    )
    .expect("seed feedback");

    let out = rerank_code(
        &conn,
        &cfg,
        &[hit(a, 1.0), hit(b, 1.0)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out[0].symbol_id, b, "downvoted symbol must sink");
    assert!(by_id(&out, a).parts.feedback < 1.0);
    assert!((by_id(&out, b).parts.feedback - 1.0).abs() < 1e-12);
}

#[test]
fn working_set_affinity_boosts_co_changed_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo_root = dir.path().join("repo");
    git_setup::init_repo(&repo_root);
    git_setup::commit_files(
        &repo_root,
        &[("a.rs", "fn a() {}\n"), ("b.rs", "fn b() {}\n")],
        "init",
    );
    // Make W dirty: written into the working tree, never committed.
    std::fs::write(repo_root.join("w.rs"), "fn w() {}\n").expect("write dirty file");

    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let cfg = Config::defaults();
    let sym_a = seed(&conn, "demo", "a.rs", "a::run", (1, 10), None);
    let sym_b = seed(&conn, "demo", "b.rs", "b::run", (1, 10), None);
    // Candidate file A co-changed with working-set file W; B has no edge.
    conn.execute(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, weight, created_at)
         VALUES ('file','file:demo:a.rs','file','file:demo:w.rs','co_changed',5,
                 '2026-06-09T00:00:00Z')",
        [],
    )
    .expect("seed co_changed edge");

    let ws = working_set(&repo_root, "demo");
    assert!(
        ws.files().iter().any(|f| f == "file:demo:w.rs"),
        "dirty file must be in the working set, got {:?}",
        ws.files()
    );

    let out = rerank_code(&conn, &cfg, &[hit(sym_a, 1.0), hit(sym_b, 1.0)], &ws).expect("rerank");
    assert_eq!(out[0].symbol_id, sym_a, "co-changed file must outrank twin");
    assert!(by_id(&out, sym_a).parts.affinity > 1.0);
    assert!((by_id(&out, sym_b).parts.affinity - 1.0).abs() < 1e-12);
}

#[test]
fn working_set_includes_recently_committed_files() {
    // Clean repo, nothing dirty: the last-N-first-parent-commits window
    // alone must pull the committed file into the working set.
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = git_setup::build_sample_repo(dir.path());
    let ws = working_set(&repo, "demo");
    assert!(
        ws.files().iter().any(|f| f == "file:demo:src.rs"),
        "committed file must be in the working set, got {:?}",
        ws.files()
    );
}

#[test]
fn working_set_skips_mega_commit_diffs() {
    // A formatting-sweep-sized commit (more files than
    // cochange::MEGA_COMMIT_FILE_CAP) inside the last-N-commits window
    // must NOT flood the working set; a normal-sized commit in the same
    // window still contributes its files.
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    git_setup::init_repo(&repo);
    git_setup::commit_files(&repo, &[("kept.rs", "fn kept() {}\n")], "small change");
    let sweep: Vec<(String, String)> = (0..25)
        .map(|i| (format!("sweep/f{i}.rs"), format!("fn f{i}() {{}}\n")))
        .collect();
    let sweep_refs: Vec<(&str, &str)> = sweep
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    git_setup::commit_files(&repo, &sweep_refs, "formatting sweep");

    let ws = working_set(&repo, "demo");
    assert!(
        ws.files().iter().any(|f| f == "file:demo:kept.rs"),
        "normal commit's file must be in the working set, got {:?}",
        ws.files()
    );
    assert!(
        !ws.files().iter().any(|f| f.contains("sweep/")),
        "mega-commit files must be skipped, got {:?}",
        ws.files()
    );
}

#[test]
fn working_set_outside_any_repo_is_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = working_set(&dir.path().join("not-a-repo"), "demo");
    assert!(ws.files().is_empty());
}

#[test]
fn chunks_coalesce_onto_parent_identity() {
    let (_d, conn) = open_db();
    let cfg = Config::defaults();
    let parent = seed(&conn, "demo", "big.rs", "big_fn", (1, 100), None);
    let c0 = seed(&conn, "demo", "big.rs", "big_fn#0", (1, 50), Some(parent));
    let c1 = seed(&conn, "demo", "big.rs", "big_fn#1", (51, 100), Some(parent));

    let out = rerank_code(
        &conn,
        &cfg,
        &[hit(c0, 1.0), hit(c1, 0.5)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out.len(), 1, "two chunks of one parent → one output row");
    let row = &out[0];
    assert_eq!(row.symbol_id, parent, "output carries the parent id");
    assert_eq!(row.symbol, "big_fn", "output carries the parent symbol");
    assert_eq!(row.kind, "function");
    assert_eq!(
        (row.line_start, row.line_end),
        (1, 50),
        "output keeps the winning chunk's line range"
    );
    assert!(
        (f64::from(row.parts.relevance) - 1.0).abs() < 1e-6,
        "output keeps the best chunk's score"
    );
}

#[test]
fn parent_and_chunks_in_one_pool_coalesce_to_single_row() {
    let (_d, conn) = open_db();
    let cfg = Config::defaults();
    let parent = seed(&conn, "demo", "big.rs", "big_fn", (1, 100), None);
    let c0 = seed(&conn, "demo", "big.rs", "big_fn#0", (1, 50), Some(parent));
    let c1 = seed(&conn, "demo", "big.rs", "big_fn#1", (51, 100), Some(parent));

    // Chunk c1 carries the highest route score: the group must collapse
    // to one row with the parent's identity but c1's line range.
    let out = rerank_code(
        &conn,
        &cfg,
        &[hit(parent, 0.6), hit(c0, 0.4), hit(c1, 1.0)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out.len(), 1, "parent + two chunks → one output row");
    assert_eq!(out[0].symbol_id, parent, "output carries the parent id");
    assert_eq!(out[0].symbol, "big_fn", "output carries the parent symbol");
    assert_eq!(
        (out[0].line_start, out[0].line_end),
        (51, 100),
        "output keeps the winning chunk's line range"
    );

    // Parent-wins variant: bump the parent's route score highest and the
    // single output row must keep the parent's OWN line range — no
    // identity swap onto a chunk's narrower span.
    let out = rerank_code(
        &conn,
        &cfg,
        &[hit(parent, 1.0), hit(c0, 0.5), hit(c1, 0.4)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out.len(), 1, "parent + two chunks → one output row");
    assert_eq!(out[0].symbol_id, parent, "output carries the parent id");
    assert_eq!(out[0].symbol, "big_fn", "output carries the parent symbol");
    assert_eq!(
        (out[0].line_start, out[0].line_end),
        (1, 100),
        "winning parent keeps its own line range"
    );
}

#[test]
fn score_parts_product_invariant() {
    let (_d, conn) = open_db();
    let cfg = Config::defaults();
    let a = seed(&conn, "demo", "a.rs", "a::run", (1, 10), None);
    let b = seed(&conn, "demo", "b.rs", "b::run", (1, 10), None);
    // Make every prior non-trivial for `a`.
    conn.execute(
        "UPDATE code_symbols SET rank_score = 0.7, access_count = 9 WHERE id = ?1",
        [a],
    )
    .expect("bump signals");
    conn.execute(
        "UPDATE code_symbols SET rank_score = 0.3 WHERE id = ?1",
        [b],
    )
    .expect("set rank");
    conn.execute(
        "INSERT INTO code_feedback(repo, path, symbol, used_count, irrelevant_count) \
         VALUES ('demo', 'a.rs', 'a::run', 6, 1)",
        [],
    )
    .expect("seed feedback");

    let out = rerank_code(
        &conn,
        &cfg,
        &[hit(a, 8.0), hit(b, 2.0)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out.len(), 2);
    for r in &out {
        let product = f64::from(r.parts.relevance)
            * r.parts.rank
            * r.parts.activation
            * r.parts.affinity
            * r.parts.feedback;
        assert!(
            (r.parts.final_score - product).abs() < 1e-6,
            "invariant broken for {}: final {} vs product {}",
            r.symbol_id,
            r.parts.final_score,
            product
        );
        // Empty working set → affinity exactly neutral.
        assert!((r.parts.affinity - 1.0).abs() < 1e-12);
    }
}

#[test]
fn vanished_rows_are_dropped() {
    let (_d, conn) = open_db();
    let cfg = Config::defaults();
    let a = seed(&conn, "demo", "a.rs", "a::run", (1, 10), None);
    let out = rerank_code(
        &conn,
        &cfg,
        &[hit(a, 1.0), hit(9_999, 1.0)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].symbol_id, a);
    assert_eq!(out[0].source, Source::Lexical);
    assert_eq!(
        (out[0].repo.as_str(), out[0].path.as_str()),
        ("demo", "a.rs")
    );
    assert_eq!(out[0].lang, "rust");
}
