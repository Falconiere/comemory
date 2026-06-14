//! Tests for [`comemory::retrieval::code_rerank`].
//!
//! Seeds real `code_symbols` / `code_feedback` / `edges` rows through the
//! production writers (`code_row::insert` + direct updates) and, for the
//! working-set affinity case, a real on-disk git repository. Asserts that
//! the four bounded priors (PageRank, activation, affinity, feedback)
//! reorder candidates deterministically and that cAST chunk rows coalesce
//! onto their parent identity.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
#[path = "common/git_sample.rs"]
mod git_sample;

#[path = "common/code_rerank_support.rs"]
mod support;

use comemory::config::Config;
use comemory::retrieval::code_rerank::{CodeReranked, WorkingSet, rerank_code, working_set};
use comemory::store::connection;

/// Find the reranked entry for `id`, panicking if the rerank dropped it —
/// only this binary inspects per-symbol output, so the helper lives here
/// rather than in the shared `code_rerank_support` module.
fn by_id(out: &[CodeReranked], id: i64) -> &CodeReranked {
    out.iter()
        .find(|r| r.symbol_id == id)
        .expect("symbol present in output")
}

#[test]
fn higher_rank_score_wins_equal_relevance() {
    let (_d, conn) = support::open_db();
    let cfg = Config::defaults();
    let central = support::seed(&conn, "demo", "core.rs", "core::run", (1, 10), None);
    let leaf = support::seed(&conn, "demo", "leaf.rs", "leaf::run", (1, 10), None);
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
        &[support::hit(central, 1.0), support::hit(leaf, 1.0)],
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
    let (_d, conn) = support::open_db();
    let cfg = Config::defaults();
    let a = support::seed(&conn, "demo", "a.rs", "a::run", (1, 10), None);
    let b = support::seed(&conn, "demo", "b.rs", "b::run", (1, 10), None);
    let out = rerank_code(
        &conn,
        &cfg,
        &[support::hit(a, 1.0), support::hit(b, 1.0)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    for r in &out {
        assert!((r.parts.rank - 1.0).abs() < 1e-12, "got {}", r.parts.rank);
    }
}

#[test]
fn access_count_bump_flips_near_tie() {
    let (_d, conn) = support::open_db();
    let cfg = Config::defaults();
    let a = support::seed(&conn, "demo", "a.rs", "a::run", (1, 10), None);
    let b = support::seed(&conn, "demo", "b.rs", "b::run", (1, 10), None);

    // Baseline: identical priors → tie breaks on ascending symbol_id.
    let out = rerank_code(
        &conn,
        &cfg,
        &[support::hit(a, 1.0), support::hit(b, 1.0)],
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
        &[support::hit(a, 1.0), support::hit(b, 1.0)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out[0].symbol_id, b, "accessed symbol must outrank its twin");
    assert!(by_id(&out, b).parts.activation > 1.0);
}

#[test]
fn irrelevant_heavy_symbol_sinks_below_neutral_twin() {
    let (_d, conn) = support::open_db();
    let cfg = Config::defaults();
    let a = support::seed(&conn, "demo", "a.rs", "a::run", (1, 10), None);
    let b = support::seed(&conn, "demo", "b.rs", "b::run", (1, 10), None);
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
        &[support::hit(a, 1.0), support::hit(b, 1.0)],
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
    git_repo::init_repo(&repo_root);
    git_commit::commit_files(
        &repo_root,
        &[("a.rs", "fn a() {}\n"), ("b.rs", "fn b() {}\n")],
        "init",
    );
    // Make W dirty: written into the working tree, never committed.
    std::fs::write(repo_root.join("w.rs"), "fn w() {}\n").expect("write dirty file");

    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let cfg = Config::defaults();
    let sym_a = support::seed(&conn, "demo", "a.rs", "a::run", (1, 10), None);
    let sym_b = support::seed(&conn, "demo", "b.rs", "b::run", (1, 10), None);
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

    let out = rerank_code(
        &conn,
        &cfg,
        &[support::hit(sym_a, 1.0), support::hit(sym_b, 1.0)],
        &ws,
    )
    .expect("rerank");
    assert_eq!(out[0].symbol_id, sym_a, "co-changed file must outrank twin");
    assert!(by_id(&out, sym_a).parts.affinity > 1.0);
    assert!((by_id(&out, sym_b).parts.affinity - 1.0).abs() < 1e-12);
}

#[test]
fn working_set_includes_recently_committed_files() {
    // Clean repo, nothing dirty: the last-N-first-parent-commits window
    // alone must pull the committed file into the working set.
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = git_sample::build_sample_repo(dir.path());
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
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("kept.rs", "fn kept() {}\n")], "small change");
    let sweep: Vec<(String, String)> = (0..25)
        .map(|i| (format!("sweep/f{i}.rs"), format!("fn f{i}() {{}}\n")))
        .collect();
    let sweep_refs: Vec<(&str, &str)> = sweep
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    git_commit::commit_files(&repo, &sweep_refs, "formatting sweep");

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
