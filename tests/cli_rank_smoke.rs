//! Ranking smoke tests through the real `comemory` binary: a recall@3 floor
//! over a 20-memory corpus (scored by `comemory eval` against a golden YAML
//! generated from the corpus), feedback-driven reordering, and rebuild
//! parity. All three drive the full save → search pipeline (identifier
//! tokenizer, weighted bm25, candidate pool, rerank priors, diversify)
//! end-to-end.

mod common;

// Included via `#[path]` rather than `pub mod corpus;` inside
// `tests/common/mod.rs`: the corpus is only consumed by this binary, and a
// declaration in the shared `mod.rs` would emit dead_code warnings in every
// other test binary that includes `common` (stats, prune, memory, config),
// failing the zero-warnings gate. Same pattern as `tests/common/vectors.rs`.
#[path = "common/corpus.rs"]
mod corpus;

use std::collections::HashMap;
use std::path::Path;

use assert_cmd::Command;
use comemory::simhash::{hamming64, of_body, NEAR_DUP_HAMMING};
use serde_json::Value;

use common::runner::Sandbox;
use corpus::{CORPUS, SMOKE_QUERIES};

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `data_dir`.
fn bin(data_dir: &Path) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", data_dir);
    c
}

/// Save one memory through the real binary and return its id from the
/// `--json` envelope. The advisory `duplicate_of` field (near-dup warning)
/// is intentionally ignored — saves always proceed.
fn save(data_dir: &Path, kind: &str, body: &str, tags: &str, quality: u8) -> String {
    let quality = quality.to_string();
    let assert = bin(data_dir)
        .args([
            "--json",
            "save",
            body,
            "--kind",
            kind,
            "--tags",
            tags,
            "--quality",
            &quality,
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let v: Value = serde_json::from_str(stdout.trim()).expect("save --json envelope");
    v["id"].as_str().expect("save id field").to_string()
}

/// Save every corpus row and return an id → body map. Bodies are resolved
/// from the save-time capture (the id comes straight from `save --json`),
/// so no `list --json` / markdown round-trip is needed.
fn save_corpus(data_dir: &Path, items: &[(&str, &str, &str, u8)]) -> HashMap<String, String> {
    let mut bodies = HashMap::new();
    for (kind, body, tags, quality) in items {
        let id = save(data_dir, kind, body, tags, *quality);
        bodies.insert(id, (*body).to_string());
    }
    bodies
}

/// Run `comemory search <query> --k 3 --json` and return the hit ids in
/// final pipeline order. Shared by all three tests.
fn top_ids(data_dir: &Path, query: &str) -> Vec<String> {
    let assert = bin(data_dir)
        .args(["--json", "search", query, "--k", "3"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let v: Value = serde_json::from_str(stdout.trim()).expect("search --json envelope");
    v["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["memory_id"].as_str().expect("memory_id").to_string())
        .collect()
}

/// The corpus must contain exactly one deliberate near-duplicate pair (the
/// two `cargo nextest` notes, SimHash Hamming 6 ≤ NEAR_DUP_HAMMING) and no
/// accidental ones — an accidental pair would silently collapse a smoke
/// query's target in the diversify stage.
#[test]
fn corpus_contains_exactly_one_near_duplicate_pair() {
    let hashes: Vec<u64> = CORPUS.iter().map(|(_, body, _, _)| of_body(body)).collect();
    let mut pairs = Vec::new();
    for i in 0..hashes.len() {
        for j in (i + 1)..hashes.len() {
            if hamming64(hashes[i], hashes[j]) <= NEAR_DUP_HAMMING {
                pairs.push((i, j));
            }
        }
    }
    assert_eq!(
        pairs.len(),
        1,
        "expected exactly one near-dup pair in the corpus, got {pairs:?}"
    );
}

/// Recall@3 floor, scored by the real `comemory eval` command. The golden
/// YAML is generated from the corpus at test time (ids are content-derived,
/// so a checked-in file would rot on body edits) and fed via
/// `--golden --golden-only`. Each query's relevant set is exactly one id
/// (enforced by `corpus::golden_pairs`), so the report's mean `recall_at_k`
/// reaches 1.0 iff every expected body lands in the top-3 — the identical
/// bar the previous hand-rolled loop asserted. Per-query misses are dumped
/// from the report so a regression shows every failure at once.
#[test]
fn recall_at_3_floor_over_smoke_corpus() {
    let sandbox = Sandbox::new();
    let dir = sandbox.data_dir();
    let bodies = save_corpus(&dir, CORPUS);
    assert_eq!(
        bodies.len(),
        CORPUS.len(),
        "duplicate ids detected: corpus contains bodies with the same SHA-256 prefix"
    );

    let pairs = corpus::golden_pairs(&bodies);
    let golden_path = sandbox.root.path().join("golden.yaml");
    let yaml = serde_yaml::to_string(&pairs).expect("serialize golden pairs to YAML");
    std::fs::write(&golden_path, yaml).expect("write generated golden.yaml");

    let assert = bin(&dir)
        .args(["--json", "eval", "--golden"])
        .arg(&golden_path)
        .args(["--golden-only", "--k", "3"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let report: Value = serde_json::from_str(stdout.trim()).expect("eval --json report");

    let results = report["results"].as_array().expect("results array");
    assert_eq!(
        results.len(),
        SMOKE_QUERIES.len(),
        "eval must score every smoke query exactly once"
    );
    let failures: Vec<String> = results
        .iter()
        .filter(|r| r["recall"].as_f64().expect("recall field") < 1.0)
        .map(|r| {
            let got: Vec<String> = r["returned"]
                .as_array()
                .expect("returned array")
                .iter()
                .take(3)
                .map(|id| {
                    let id = id.as_str().expect("returned id");
                    let body = bodies.get(id).map(String::as_str).unwrap_or("<unknown id>");
                    format!("{id}: {body}")
                })
                .collect();
            format!(
                "query {:?}: relevant {:?} not in top-3:\n    {}",
                r["query"],
                r["relevant"],
                got.join("\n    ")
            )
        })
        .collect();
    let recall_at_3 = report["recall_at_k"].as_f64().expect("recall_at_k field");
    assert!(
        recall_at_3 >= 1.0,
        "recall@3 floor failed (mean {:.3}) for {}/{} queries:\n{}",
        recall_at_3,
        failures.len(),
        SMOKE_QUERIES.len(),
        failures.join("\n")
    );
}

/// Three `--irrelevant` votes drive the leader's Beta feedback to
/// `(0+1)/(3+4) = 1/7`, mapping to a `1/7 / 0.25 ≈ 0.571` multiplier vs the
/// untouched memory's neutral `1.0` — a far larger gap than the bm25
/// difference between two near-equal-relevance bodies, so the leader must
/// lose the top spot.
#[test]
fn irrelevant_feedback_reorders_results() {
    let sandbox = Sandbox::new();
    let dir = sandbox.data_dir();
    let body_a = "sqlite busy timeout fix for the connection pool";
    let body_b = "sqlite busy timeout workaround for pool checkout";
    // Guard: the two memories must NOT collapse as near-duplicates in the
    // diversify stage or only one would survive to be reordered.
    // (Measured Hamming: 21.)
    assert!(
        hamming64(of_body(body_a), of_body(body_b)) > NEAR_DUP_HAMMING,
        "test bodies must not be near-duplicates"
    );
    save(&dir, "bug", body_a, "", 3);
    save(&dir, "bug", body_b, "", 3);

    let before = top_ids(&dir, "sqlite busy timeout");
    assert_eq!(
        before.len(),
        2,
        "both memories must match the query: {before:?}"
    );
    let leader = before[0].clone();

    for _ in 0..3 {
        bin(&dir)
            .args(["feedback", "q-20260610-aabbccdd", "--irrelevant", &leader])
            .assert()
            .success();
    }

    let after = top_ids(&dir, "sqlite busy timeout");
    assert_eq!(after.len(), 2, "both memories must still match: {after:?}");
    assert_ne!(
        after[0], leader,
        "irrelevant feedback must demote the previous leader (before: {before:?}, after: {after:?})"
    );
    assert!(
        after.contains(&leader),
        "demoted leader must still be returned, not dropped: {after:?}"
    );
}

/// `comemory rebuild` must not change lexical ranking. Ordered equality is
/// deterministic here even though searches bump `access_count` and rebuild
/// resets it, because the two queries' top-3 sets are *disjoint* (asserted
/// below, so corpus drift is caught): no memory is ever bumped twice, so
/// every access count seen at scoring time is 0 or 1 — and activation is
/// `ln(max(n, 1))`, which yields exactly 0 for both. The bumps are therefore
/// score-invisible regardless of where `record_access` runs relative to
/// scoring. No feedback is recorded (and rebuild wipes the feedback table
/// anyway), and `created_at` survives the rebuild via frontmatter, so every
/// score input is bit-for-bit comparable across the swap.
#[test]
fn rebuild_preserves_search_results() {
    let sandbox = Sandbox::new();
    let dir = sandbox.data_dir();
    // Entries 0..6 (pgbouncer, vec-blob DDL, VecDimMismatch, CLI --json,
    // bm25 sign, tracing::warn) chosen so q1/q2 below have disjoint top-3
    // sets; the disjointness assertion guards future corpus edits.
    save_corpus(&dir, &CORPUS[..6]);

    // q1 resolves via the strict AND tier (only the pgbouncer memory matches
    // all terms); q2 deliberately has no single memory matching all terms,
    // falling through to the relaxed OR tier where several memories compete
    // — a meaningful ordering to preserve. q2's terms avoid "postgres" so
    // its OR tier cannot pull in q1's hit: the result sets stay disjoint.
    let q1 = "postgres pool exhausted";
    let q2 = "sqlite fts5 vectors";

    let before1 = top_ids(&dir, q1);
    let before2 = top_ids(&dir, q2);
    assert!(!before1.is_empty(), "q1 must hit before rebuild");
    assert!(
        before2.len() >= 2,
        "q2 must rank multiple competitors before rebuild: {before2:?}"
    );
    // Disjointness underpins the counts-≤-1 → activation-0 invariant in the
    // doc comment; if a corpus edit ever makes the sets overlap, fail loudly
    // here instead of silently leaning on BM25 margins.
    assert!(
        before1.iter().all(|id| !before2.contains(id)),
        "q1/q2 top-3 sets must be disjoint (q1: {before1:?}, q2: {before2:?})"
    );

    bin(&dir).args(["rebuild"]).assert().success();

    assert_eq!(before1, top_ids(&dir, q1), "rebuild changed the q1 ranking");
    assert_eq!(before2, top_ids(&dir, q2), "rebuild changed the q2 ranking");
}
