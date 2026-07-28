//! Tests for [`comemory::eval::tune`] — grid shape, the honesty floor,
//! report determinism over a real db, and atomic config.toml apply.

use comemory::config::Config;
use comemory::config::file::TuneConfig;
use comemory::errors::Error;
use comemory::eval::golden::GoldenPair;
use comemory::eval::tune::{self, ScoredCandidate, TuneCandidate, TuneReport};

/// Ten lexically distinct (id, body) rows so each golden query
/// discriminates a single memory.
const TOPICS: &[(&str, &str)] = &[
    ("aaaa0001", "postgres advisory lock migration ordering"),
    ("aaaa0002", "tokio runtime shutdown sequencing bug"),
    ("aaaa0003", "clap derive global flag placement"),
    ("aaaa0004", "sqlite fts5 tokenizer unicode normalization"),
    ("aaaa0005", "docker compose volume mount permissions"),
    ("aaaa0006", "kubernetes ingress certificate renewal"),
    ("aaaa0007", "redis cache eviction policy tuning"),
    ("aaaa0008", "graphql federation gateway timeout"),
    ("aaaa0009", "webpack chunk splitting heuristics"),
    ("aaaa000a", "terraform state locking dynamodb"),
];

/// Build a real db with the [`TOPICS`] corpus plus one golden pair per
/// topic (query = body, relevant = [id]). Returns the tempdir guard, the
/// connection, and the pairs.
fn seeded() -> (tempfile::TempDir, rusqlite::Connection, Vec<GoldenPair>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    let mut pairs = Vec::with_capacity(TOPICS.len());
    for (i, (id, body)) in TOPICS.iter().enumerate() {
        conn.execute(
            "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                                  body, created_at, updated_at, md_path, simhash)
             VALUES (?1, ?1, 'note', 'd', 'f', 3, 1, ?1, ?2,
                     '2026-06-09T00:00:00Z', '2026-06-09T00:00:00Z', ?1, ?3)",
            rusqlite::params![id, body, i as i64],
        )
        .expect("insert memory");
        conn.execute(
            "INSERT INTO memory_fts(memory_id, body, tags) VALUES (?1, ?2, '')",
            rusqlite::params![id, body],
        )
        .expect("insert fts");
        pairs.push(GoldenPair {
            query: (*body).into(),
            relevant: vec![(*id).into()],
            repo: None,
            kind: None,
        });
    }
    (dir, conn, pairs)
}

/// Assert every pair of grid points differs.
fn assert_pairwise_distinct(g: &[TuneCandidate]) {
    for (i, a) in g.iter().enumerate() {
        for b in &g[i + 1..] {
            assert_ne!(a, b, "grid points must be pairwise distinct");
        }
    }
}

#[test]
fn default_grid_is_729_distinct_configs() {
    // The legacy 3^4 = 81-point grid, widened by the two graph pools to
    // 3^6 = 729. Those two are the OUTERMOST dimensions, so the grid is
    // 9 consecutive 81-point blocks, one per (hops, seeds) pair.
    let g = tune::grid(&Config::defaults().tune);
    assert_eq!(g.len(), 729, "default 3^6 grid must have 729 points");
    assert_pairwise_distinct(&g);
    assert!(
        g[..81]
            .iter()
            .all(|c| c.graph_hops == 0 && c.graph_seeds == 4),
        "the first block must hold the first (hops, seeds) pair"
    );
    assert!(
        g[81..162]
            .iter()
            .all(|c| c.graph_hops == 0 && c.graph_seeds == 8),
        "graph_seeds is the inner of the two new dimensions"
    );
    assert!(
        g[243..324]
            .iter()
            .all(|c| c.graph_hops == 1 && c.graph_seeds == 4),
        "graph_hops is the outermost dimension"
    );
}

#[test]
fn singleton_graph_pools_reproduce_the_legacy_81_sequence() {
    // Legacy parity: with one hop value and one seed value, the grid is
    // exactly the pre-F5 4-loop nest (rrf_k → decay → mmr_lambda → bm25),
    // in the same order, carrying the singleton graph knobs.
    let d = TuneConfig::default();
    let t = TuneConfig {
        graph_hops_grid: vec![0],
        graph_seeds_grid: vec![4],
        ..d.clone()
    };
    let mut expected = Vec::with_capacity(81);
    for &rrf_k in &d.rrf_k_grid {
        for &decay in &d.decay_grid {
            for &mmr_lambda in &d.mmr_lambda_grid {
                for &bm25_weights in &d.bm25_grid {
                    expected.push((rrf_k, decay, mmr_lambda, bm25_weights));
                }
            }
        }
    }
    let g = tune::grid(&t);
    assert_eq!(g.len(), 81);
    let projected: Vec<_> = g
        .iter()
        .map(|c| (c.rrf_k, c.decay, c.mmr_lambda, c.bm25_weights))
        .collect();
    assert_eq!(projected, expected, "legacy field order must be preserved");
    assert!(g.iter().all(|c| c.graph_hops == 0 && c.graph_seeds == 4));
}

#[test]
fn grid_len_is_the_product_of_the_configured_lists() {
    // The grid is the cartesian product of the six configured lists, so
    // its length is the product of their lengths — 1 × 2 × 1 × 2 × 2 × 1 = 8.
    let t = TuneConfig {
        rrf_k_grid: vec![60.0],
        decay_grid: vec![0.3, 0.5],
        mmr_lambda_grid: vec![0.7],
        bm25_grid: vec![(1.0, 3.0), (2.0, 1.0)],
        graph_hops_grid: vec![0, 2],
        graph_seeds_grid: vec![8],
        samples: 0,
    };
    let g = tune::grid(&t);
    assert_eq!(g.len(), 8);
    assert_pairwise_distinct(&g);
    assert!(
        g.contains(&TuneCandidate {
            rrf_k: 60.0,
            decay: 0.5,
            mmr_lambda: 0.7,
            bm25_weights: (2.0, 1.0),
            graph_hops: 2,
            graph_seeds: 8,
        }),
        "every list combination must appear in the product"
    );
}

#[test]
fn tune_refuses_thin_golden_set() {
    let (_d, conn, pairs) = seeded();
    let cfg = Config::defaults();
    let thin = pairs[..3].to_vec();
    let err = tune::run_tune(&cfg, &conn, &thin, 3, 10, None).expect_err("3 pairs must refuse");
    assert!(
        matches!(err, Error::Unavailable(_)),
        "thin golden set must surface Error::Unavailable, got {err:?}"
    );
}

#[test]
fn tune_is_deterministic() {
    // Default config samples (64 of the 729 grid points); with no --seed
    // the seed is derived from inputs that did not change, so two runs
    // must agree candidate-for-candidate.
    let (_d, conn, pairs) = seeded();
    let cfg = Config::defaults();
    let r1 = tune::run_tune(&cfg, &conn, &pairs, 3, 10, None).expect("first run");
    let r2 = tune::run_tune(&cfg, &conn, &pairs, 3, 10, None).expect("second run");
    assert_eq!(r1.ranked.len(), cfg.tune.samples);
    assert_eq!(
        serde_json::to_string(&r1).expect("serialize first"),
        serde_json::to_string(&r2).expect("serialize second"),
        "two tune runs over the same db must be byte-identical"
    );
}

#[test]
fn tune_reports_are_seed_reproducible_and_seed_sensitive() {
    let (_d, conn, pairs) = seeded();
    let cfg = Config::defaults();
    let run = |seed: u64| {
        let r = tune::run_tune(&cfg, &conn, &pairs, 3, 10, Some(seed)).expect("seeded run");
        serde_json::to_string(&r).expect("serialize report")
    };
    assert_eq!(run(42), run(42), "one seed must reproduce one report");
    assert_ne!(
        run(42),
        run(7),
        "a different seed must draw a different candidate set"
    );
}

#[test]
fn samples_zero_evaluates_the_whole_grid() {
    // The escape hatch back to exhaustive search: every grid point scored,
    // no sampling, `--seed` irrelevant.
    let (_d, conn, pairs) = seeded();
    let mut cfg = Config::defaults();
    cfg.tune = TuneConfig {
        rrf_k_grid: vec![20.0, 60.0],
        decay_grid: vec![0.5],
        mmr_lambda_grid: vec![0.7],
        bm25_grid: vec![(1.0, 3.0)],
        graph_hops_grid: vec![0, 2],
        graph_seeds_grid: vec![8],
        samples: 0,
    };
    let r = tune::run_tune(&cfg, &conn, &pairs, 3, 10, Some(42)).expect("exhaustive run");
    assert_eq!(r.ranked.len(), 4, "2 × 1 × 1 × 1 × 2 × 1 grid points");
    let ranked: Vec<_> = r.ranked.iter().map(|s| s.candidate).collect();
    for c in tune::grid(&cfg.tune) {
        assert!(
            ranked.contains(&c),
            "every grid point must be scored: {c:?}"
        );
    }
}

/// Build a [`ScoredCandidate`] with fixed knobs and the given scores.
fn scored(mrr: f64, recall_at_k: f64) -> ScoredCandidate {
    ScoredCandidate {
        candidate: TuneCandidate {
            rrf_k: 60.0,
            decay: 0.5,
            mmr_lambda: 0.7,
            bm25_weights: (1.0, 3.0),
            graph_hops: 2,
            graph_seeds: 8,
        },
        mrr,
        recall_at_k,
    }
}

/// Build a [`TuneReport`] whose ranking carries exactly `winner`.
fn report_with(baseline: ScoredCandidate, winner: ScoredCandidate) -> TuneReport {
    TuneReport {
        k: 3,
        golden_pairs: 10,
        baseline,
        ranked: vec![winner],
    }
}

#[test]
fn winner_is_the_top_ranked_candidate() {
    let report = report_with(scored(0.5, 0.5), scored(0.9, 0.7));
    let w = report.winner().expect("non-empty ranking has a winner");
    assert!((w.mrr - 0.9).abs() < f64::EPSILON);

    let empty = TuneReport {
        k: 3,
        golden_pairs: 10,
        baseline: scored(0.5, 0.5),
        ranked: vec![],
    };
    assert!(empty.winner().is_err(), "empty ranking must error");
}

#[test]
fn improves_baseline_requires_a_strict_win() {
    // Higher mrr wins outright.
    assert!(report_with(scored(0.5, 0.9), scored(0.6, 0.1)).improves_baseline());
    // Exact mrr tie: recall@k breaks it, strictly.
    assert!(report_with(scored(0.5, 0.5), scored(0.5, 0.6)).improves_baseline());
    // Full tie is NOT an improvement — --apply must not churn config.toml.
    assert!(!report_with(scored(0.5, 0.5), scored(0.5, 0.5)).improves_baseline());
    // Lower mrr never improves, regardless of recall.
    assert!(!report_with(scored(0.5, 0.1), scored(0.4, 1.0)).improves_baseline());
}

#[test]
fn resolve_min_pairs_reads_the_env_hook() {
    // Unset: the documented floor.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_TUNE_MIN_GOLDEN") };
    assert_eq!(
        tune::resolve_min_pairs().expect("default"),
        tune::MIN_GOLDEN_PAIRS
    );
    // Set: the test hook lowers (or raises) the floor.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_TUNE_MIN_GOLDEN", "3") };
    let lowered = tune::resolve_min_pairs();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_TUNE_MIN_GOLDEN", "not-a-number") };
    let invalid = tune::resolve_min_pairs();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_TUNE_MIN_GOLDEN") };
    assert_eq!(lowered.expect("valid override"), 3);
    let msg = invalid
        .expect_err("invalid override must error")
        .to_string();
    assert!(
        msg.contains("COMEMORY_TUNE_MIN_GOLDEN"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn apply_writes_atomic_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "embed_hint = \"x\"\n").expect("write config");
    let w = TuneCandidate {
        rrf_k: 20.0,
        decay: 0.3,
        mmr_lambda: 0.9,
        bm25_weights: (2.0, 1.0),
        graph_hops: 1,
        graph_seeds: 16,
    };
    tune::apply_to_config_file(&path, &w).expect("apply");
    let cfg = Config::defaults()
        .with_file(&path)
        .expect("reload written config");
    assert_eq!(cfg.retrieval.rrf_k, 20.0);
    assert_eq!(cfg.retrieval.bm25_weights, (2.0, 1.0));
    assert_eq!(cfg.rank.decay, 0.3);
    assert_eq!(cfg.rank.mmr_lambda, 0.9);
    // The graph knobs are written as TOML integers, so the reload parses
    // them back into u32 / usize rather than failing on a float.
    assert_eq!(cfg.retrieval.graph_hops, 1);
    assert_eq!(cfg.retrieval.graph_seeds, 16);
    assert_eq!(
        cfg.embed_hint.as_deref(),
        Some("x"),
        "pre-existing keys must survive the apply"
    );
    assert!(
        !dir.path().join("config.toml.tmp").exists(),
        "tmp staging file must be renamed away"
    );
}
