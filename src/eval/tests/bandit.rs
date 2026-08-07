#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::eval::bandit`] — arm ids, seeding, Thompson sample
//! determinism, posterior updates, and the shared `beats_baseline` gate.
//! Real temp db via `connection::open`; no mocks.

use std::fmt::Write as _;

use comemory::config::{Config, TuneConfig};
use comemory::eval::bandit::{self, Arm};
use comemory::eval::golden::GoldenPair;
use comemory::eval::tune::{self, TuneCandidate};
use comemory::store::connection;
use sha2::{Digest, Sha256};

fn tiny_cfg() -> Config {
    let mut cfg = Config::defaults();
    cfg.tune = TuneConfig {
        rrf_k_grid: vec![60.0],
        decay_grid: vec![0.5],
        mmr_lambda_grid: vec![0.7],
        bm25_grid: vec![(1.0, 3.0)],
        graph_hops_grid: vec![2],
        graph_seeds_grid: vec![8],
        samples: 0,
    };
    cfg
}

fn cand() -> TuneCandidate {
    TuneCandidate {
        rrf_k: 60.0,
        decay: 0.5,
        mmr_lambda: 0.7,
        bm25_weights: (1.0, 3.0),
        graph_hops: 2,
        graph_seeds: 8,
    }
}

/// The pre-F5 arm id: SHA-256 over the five legacy knob bit patterns only.
/// Rows written under this scheme are what a database upgraded into F5
/// carries.
fn legacy_arm_id(c: &TuneCandidate) -> String {
    let mut h = Sha256::new();
    h.update(c.rrf_k.to_bits().to_le_bytes());
    h.update(c.decay.to_bits().to_le_bytes());
    h.update(c.mmr_lambda.to_bits().to_le_bytes());
    h.update(c.bm25_weights.0.to_bits().to_le_bytes());
    h.update(c.bm25_weights.1.to_bits().to_le_bytes());
    h.finalize()[..8].iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Ten lexically distinct rows plus one golden pair each, so `run_bandit`
/// has a real corpus to confirm against.
fn seeded_corpus(conn: &rusqlite::Connection) -> Vec<GoldenPair> {
    const TOPICS: &[(&str, &str)] = &[
        ("bbbb0001", "postgres advisory lock migration ordering"),
        ("bbbb0002", "tokio runtime shutdown sequencing bug"),
        ("bbbb0003", "clap derive global flag placement"),
    ];
    let mut pairs = Vec::with_capacity(TOPICS.len());
    for (i, (id, body)) in TOPICS.iter().enumerate() {
        conn.execute(
            "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                                  body, created_at, updated_at, md_path, simhash)
             VALUES (?1, ?1, 'note', 'd', 'f', 3, 1, ?1, ?2,
                     '2026-07-20T00:00:00Z', '2026-07-20T00:00:00Z', ?1, ?3)",
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
    pairs
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
                ..cand()
            },
            alpha: 1.0,
            beta: 1.0,
            pulls: 0,
            last_mrr: None,
        },
        Arm {
            arm_id: "b".into(),
            candidate: cand(),
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
fn graph_knobs_alone_produce_distinct_arm_ids() {
    // The graph pair joined the hash in F5: two candidates that differ
    // only there are different arms, not one arm double-counted.
    let base = cand();
    let more_hops = TuneCandidate {
        graph_hops: 1,
        ..base
    };
    let more_seeds = TuneCandidate {
        graph_seeds: 16,
        ..base
    };
    let ids = [
        bandit::arm_id(&base),
        bandit::arm_id(&more_hops),
        bandit::arm_id(&more_seeds),
    ];
    assert_ne!(ids[0], ids[1], "graph_hops must reach the hash");
    assert_ne!(ids[0], ids[2], "graph_seeds must reach the hash");
    assert_ne!(ids[1], ids[2]);
    assert_ne!(
        bandit::arm_id(&base),
        legacy_arm_id(&base),
        "the widened hash must not collide with the pre-F5 scheme"
    );
}

#[test]
fn a_pre_f5_arm_row_is_orphaned_not_reused() {
    // A database upgraded into F5 still holds rows keyed by the 5-field
    // hash. They must neither be read as a current arm nor break the run:
    // seeding is INSERT OR IGNORE, ranking only reads recomputed ids.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("c.db")).expect("open");
    let pairs = seeded_corpus(&conn);
    let cfg = tiny_cfg();
    let stale = legacy_arm_id(&cand());
    conn.execute(
        "INSERT INTO bandit_arms(arm_id, rrf_k, decay, mmr_lambda, bm25_body, bm25_tags,
                                 alpha, beta, pulls, last_mrr, updated_at)
         VALUES (?1, 60.0, 0.5, 0.7, 1.0, 3.0, 9.0, 1.0, 8, 0.99, '2026-07-01T00:00:00Z')",
        [&stale],
    )
    .expect("seed a pre-F5 arm row");

    let config_path = dir.path().join("config.toml");
    let report = bandit::run_bandit(&cfg, &mut conn, &pairs, 3, 3, false, &config_path)
        .expect("run_bandit must tolerate an orphaned arm row");

    assert_eq!(report.ranked.len(), 1, "only the current grid arm ranks");
    assert_eq!(report.ranked[0].arm_id, bandit::arm_id(&cand()));
    assert_ne!(report.ranked[0].arm_id, stale);
    let (alpha, pulls): (f64, i64) = conn
        .query_row(
            "SELECT alpha, pulls FROM bandit_arms WHERE arm_id = ?1",
            [&stale],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the stale row must survive untouched");
    assert!((alpha - 9.0).abs() < f64::EPSILON);
    assert_eq!(pulls, 8, "an orphaned arm must not be updated");
}

#[test]
fn beats_baseline_matches_tune_predicate() {
    assert!(tune::beats_baseline(0.9, 0.5, 0.8, 0.9));
    assert!(tune::beats_baseline(0.8, 0.95, 0.8, 0.9));
    assert!(!tune::beats_baseline(0.8, 0.9, 0.8, 0.9));
    assert!(!tune::beats_baseline(0.7, 1.0, 0.8, 0.5));
}
