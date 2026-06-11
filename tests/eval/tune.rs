//! Tests for [`comemory::eval::tune`] — grid shape, the honesty floor,
//! report determinism over a real db, and atomic config.toml apply.

use comemory::config::file::TuneConfig;
use comemory::config::Config;
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
fn default_grid_is_81_distinct_configs() {
    let g = tune::grid(&Config::defaults().tune);
    assert_eq!(g.len(), 81, "default 3^4 grid must have 81 points");
    assert_pairwise_distinct(&g);
}

#[test]
fn grid_len_is_the_product_of_the_configured_lists() {
    // The grid is the cartesian product of the four configured lists, so
    // its length is the product of their lengths — here 1 × 2 × 1 × 2 = 4.
    let t = TuneConfig {
        rrf_k_grid: vec![60.0],
        decay_grid: vec![0.3, 0.5],
        mmr_lambda_grid: vec![0.7],
        bm25_grid: vec![(1.0, 3.0), (2.0, 1.0)],
    };
    let g = tune::grid(&t);
    assert_eq!(g.len(), 4);
    assert_pairwise_distinct(&g);
    assert!(
        g.contains(&TuneCandidate {
            rrf_k: 60.0,
            decay: 0.5,
            mmr_lambda: 0.7,
            bm25_weights: (2.0, 1.0),
        }),
        "every list combination must appear in the product"
    );
}

#[test]
fn tune_refuses_thin_golden_set() {
    let (_d, conn, pairs) = seeded();
    let cfg = Config::defaults();
    let thin = pairs[..3].to_vec();
    let err = tune::run_tune(&cfg, &conn, &thin, 3, 10).expect_err("3 pairs must refuse");
    assert!(
        matches!(err, Error::Unavailable(_)),
        "thin golden set must surface Error::Unavailable, got {err:?}"
    );
}

#[test]
fn tune_is_deterministic() {
    let (_d, conn, pairs) = seeded();
    let cfg = Config::defaults();
    let r1 = tune::run_tune(&cfg, &conn, &pairs, 3, 10).expect("first run");
    let r2 = tune::run_tune(&cfg, &conn, &pairs, 3, 10).expect("second run");
    assert_eq!(r1.ranked.len(), 81);
    assert_eq!(
        serde_json::to_string(&r1).expect("serialize first"),
        serde_json::to_string(&r2).expect("serialize second"),
        "two tune runs over the same db must be byte-identical"
    );
}

/// Build a [`ScoredCandidate`] with fixed knobs and the given scores.
fn scored(mrr: f64, recall_at_k: f64) -> ScoredCandidate {
    ScoredCandidate {
        candidate: TuneCandidate {
            rrf_k: 60.0,
            decay: 0.5,
            mmr_lambda: 0.7,
            bm25_weights: (1.0, 3.0),
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
    std::env::remove_var("COMEMORY_TUNE_MIN_GOLDEN");
    assert_eq!(
        tune::resolve_min_pairs().expect("default"),
        tune::MIN_GOLDEN_PAIRS
    );
    // Set: the test hook lowers (or raises) the floor.
    std::env::set_var("COMEMORY_TUNE_MIN_GOLDEN", "3");
    let lowered = tune::resolve_min_pairs();
    std::env::set_var("COMEMORY_TUNE_MIN_GOLDEN", "not-a-number");
    let invalid = tune::resolve_min_pairs();
    std::env::remove_var("COMEMORY_TUNE_MIN_GOLDEN");
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
    };
    tune::apply_to_config_file(&path, &w).expect("apply");
    let cfg = Config::defaults()
        .with_file(&path)
        .expect("reload written config");
    assert_eq!(cfg.retrieval.rrf_k, 20.0);
    assert_eq!(cfg.retrieval.bm25_weights, (2.0, 1.0));
    assert_eq!(cfg.rank.decay, 0.3);
    assert_eq!(cfg.rank.mmr_lambda, 0.9);
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
