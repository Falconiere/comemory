//! Tests for [`comemory::eval::bandit`] — arm ids, seeding, Thompson sample
//! determinism, posterior updates, and the shared `beats_baseline` gate.
//! Real temp db via `connection::open`; no mocks.

use comemory::config::{Config, TuneConfig};
use comemory::eval::bandit::{self, Arm};
use comemory::eval::tune::{self, TuneCandidate};
use comemory::store::connection;

fn tiny_cfg() -> Config {
    let mut cfg = Config::defaults();
    cfg.tune = TuneConfig {
        rrf_k_grid: vec![60.0],
        decay_grid: vec![0.5],
        mmr_lambda_grid: vec![0.7],
        bm25_grid: vec![(1.0, 3.0)],
    };
    cfg
}

fn cand() -> TuneCandidate {
    TuneCandidate {
        rrf_k: 60.0,
        decay: 0.5,
        mmr_lambda: 0.7,
        bm25_weights: (1.0, 3.0),
    }
}

#[test]
fn arm_id_is_stable_for_same_candidate() {
    let c = cand();
    let a = bandit::arm_id(&c);
    let b = bandit::arm_id(&c);
    assert_eq!(a, b);
    assert_eq!(a.len(), 16, "arm_id is 16 hex chars");
    assert!(a.chars().all(|ch| ch.is_ascii_hexdigit()));
    // Distinct knobs → distinct ids.
    let mut other = c;
    other.decay = 0.8;
    assert_ne!(bandit::arm_id(&c), bandit::arm_id(&other));
}

#[test]
fn seed_arms_then_load_ranked_returns_grid_priors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    let cfg = tiny_cfg();
    bandit::seed_arms(&conn, &cfg, "2026-07-20T00:00:00Z").expect("seed");
    let ranked = bandit::load_ranked(&conn, &cfg).expect("load");
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].arm_id, bandit::arm_id(&cand()));
    assert!((ranked[0].alpha - 1.0).abs() < f64::EPSILON);
    assert!((ranked[0].beta - 1.0).abs() < f64::EPSILON);
    assert_eq!(ranked[0].pulls, 0);
    assert!(ranked[0].last_mrr.is_none());
}

#[test]
fn thompson_sample_is_deterministic_for_same_seed() {
    let arms = vec![
        Arm {
            arm_id: "a".into(),
            candidate: TuneCandidate {
                rrf_k: 20.0,
                decay: 0.5,
                mmr_lambda: 0.7,
                bm25_weights: (1.0, 3.0),
            },
            alpha: 1.0,
            beta: 1.0,
            pulls: 0,
            last_mrr: None,
        },
        Arm {
            arm_id: "b".into(),
            candidate: TuneCandidate {
                rrf_k: 60.0,
                decay: 0.5,
                mmr_lambda: 0.7,
                bm25_weights: (1.0, 3.0),
            },
            alpha: 1.0,
            beta: 1.0,
            pulls: 0,
            last_mrr: None,
        },
    ];
    let seed = bandit::sample_seed(10, 2);
    let first = bandit::thompson_sample(&arms, seed).expect("sample 1");
    let second = bandit::thompson_sample(&arms, seed).expect("sample 2");
    assert_eq!(first.arm_id, second.arm_id);
    assert_eq!(first.candidate, second.candidate);
}

#[test]
fn record_outcome_bumps_alpha_on_win_and_beta_on_loss() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    let cfg = tiny_cfg();
    let at = "2026-07-20T12:00:00Z";
    bandit::seed_arms(&conn, &cfg, at).expect("seed");
    let id = bandit::arm_id(&cand());

    bandit::record_outcome(&conn, &id, true, 0.9, at).expect("win");
    let (alpha, beta, pulls, last): (f64, f64, i64, f64) = conn
        .query_row(
            "SELECT alpha, beta, pulls, last_mrr FROM bandit_arms WHERE arm_id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("after win");
    assert!((alpha - 2.0).abs() < f64::EPSILON);
    assert!((beta - 1.0).abs() < f64::EPSILON);
    assert_eq!(pulls, 1);
    assert!((last - 0.9).abs() < f64::EPSILON);

    bandit::record_outcome(&conn, &id, false, 0.4, at).expect("loss");
    let (alpha, beta, pulls, last): (f64, f64, i64, f64) = conn
        .query_row(
            "SELECT alpha, beta, pulls, last_mrr FROM bandit_arms WHERE arm_id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("after loss");
    assert!((alpha - 2.0).abs() < f64::EPSILON);
    assert!((beta - 2.0).abs() < f64::EPSILON);
    assert_eq!(pulls, 2);
    assert!((last - 0.4).abs() < f64::EPSILON);
}

#[test]
fn beats_baseline_matches_tune_predicate() {
    assert!(tune::beats_baseline(0.9, 0.5, 0.8, 0.9));
    assert!(tune::beats_baseline(0.8, 0.95, 0.8, 0.9));
    assert!(!tune::beats_baseline(0.8, 0.9, 0.8, 0.9));
    assert!(!tune::beats_baseline(0.7, 1.0, 0.8, 0.5));
}
