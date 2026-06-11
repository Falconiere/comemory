//! Tests for [`comemory::eval::tune`] — grid shape, the honesty floor,
//! report determinism over a real db, and atomic config.toml apply.

use comemory::config::Config;
use comemory::errors::Error;
use comemory::eval::golden::GoldenPair;
use comemory::eval::tune::{self, TuneCandidate};

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
        });
    }
    (dir, conn, pairs)
}

#[test]
fn grid_is_81_distinct_configs() {
    let g = tune::grid();
    assert_eq!(g.len(), 81, "3^4 grid must have 81 points");
    for (i, a) in g.iter().enumerate() {
        for b in &g[i + 1..] {
            assert_ne!(a, b, "grid points must be pairwise distinct");
        }
    }
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
