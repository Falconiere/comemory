#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Ranking invariance across the console-compat change (spec AC-22).
//!
//! Adding pre-fusion leg scores to `score_parts` must not move a single
//! result: the values are carried through from leg vectors the router
//! already holds, never recomputed. Proving that needs a record of the
//! ranking as it stood *before* the retrieval edit, and a snapshot written
//! by the same commit that edits the ranker cannot testify about the
//! ranking before it. So `tests/golden/ranking-invariance.json` is
//! generated from the pre-change binary by
//! `tests/golden/ranking-invariance.gen.sh` and committed first; this test
//! replays it.
//!
//! Real data end to end: the fixture carries the actual markdown files the
//! generator's `comemory save` runs produced, this test writes them back to
//! a temp data dir, rebuilds the real SQLite mirror from them with the real
//! binary, and runs the real queries. No mocks, and no second copy of the
//! corpus — the fixture is the single source of truth for what to rebuild.

use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;
use serde::Deserialize;
use tempfile::tempdir;

/// The committed fixture: the corpus to rebuild and the ranking to expect.
#[derive(Deserialize)]
struct Fixture {
    corpus: Vec<CorpusFile>,
    expected: Vec<ExpectedQuery>,
}

/// One memory's markdown exactly as `comemory save` wrote it.
#[derive(Deserialize)]
struct CorpusFile {
    file: String,
    markdown: String,
}

/// One query and the hit ordering it produced before the change.
#[derive(Deserialize)]
struct ExpectedQuery {
    query: String,
    hits: Vec<ExpectedHit>,
}

/// One expected hit. See [`assert_parts`] for which fields are compared
/// exactly and which carry a tolerance, and why.
#[derive(Deserialize)]
struct ExpectedHit {
    memory_id: String,
    // `score` is present in the fixture for human reference but is not
    // deserialized: it inherits the time-dependent activation factor, so it
    // is checked for internal agreement against the live hit rather than
    // against the recording. See `assert_parts`.
    tier: u8,
    score_parts: Parts,
}

/// The live `search --json` envelope, narrowed to the fields this test
/// asserts on.
#[derive(Deserialize)]
struct SearchEnvelope {
    hits: Vec<ActualHit>,
}

/// One live hit.
#[derive(Deserialize)]
struct ActualHit {
    memory_id: String,
    score: f64,
    tier: u8,
    score_parts: Parts,
}

/// The multiplicative factors behind a hit's score. Split by the test into
/// two classes — see [`assert_parts`].
#[derive(Deserialize)]
struct Parts {
    rrf: f64,
    activation: f64,
    feedback: f64,
    quality: f64,
    supersede: f64,
    rank: f64,
    final_score: f64,
}

/// Relative tolerance for `rank`, the materialized memory PageRank.
/// `restore_corpus` rebuilds it from markdown and the power iteration
/// converges along a slightly different path than the incremental refresh
/// `save` performed when the fixture was recorded — observed at ~1e-5
/// relative. 1e-3 sits far above that and far below the effect of any real
/// ranking change, which moves a score by whole percent.
const DRIFT_TOLERANCE: f64 = 1e-3;

/// Tolerance for the internal `final_score == product of its own parts`
/// identity. This is checked against a hit's OWN live values, not against
/// the fixture, so it only has to absorb f32/f64 rounding.
const PRODUCT_TOLERANCE: f64 = 1e-6;

fn fixture() -> Fixture {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/ranking-invariance.json"
    );
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {path}: {e} — regenerate with ranking-invariance.gen.sh"));
    serde_json::from_str(&raw).expect("ranking-invariance.json parses")
}

/// Write the fixture's markdown back into `data_dir/memories/` and rebuild
/// the SQLite mirror from it with the real binary — the same reconstruction
/// path `comemory rebuild` gives a user whose database was lost.
fn restore_corpus(data_dir: &Path, corpus: &[CorpusFile]) {
    let memories = data_dir.join("memories");
    fs::create_dir_all(&memories).unwrap();
    for entry in corpus {
        fs::write(memories.join(&entry.file), &entry.markdown).unwrap();
    }
    Command::cargo_bin("comemory")
        .unwrap()
        .arg("rebuild")
        .env("COMEMORY_DATA_DIR", data_dir)
        .assert()
        .success();
}

fn run_query(data_dir: &Path, query: &str) -> Vec<ActualHit> {
    let out = Command::cargo_bin("comemory")
        .unwrap()
        .args(["search", query, "--json", "--k", "8"])
        .env("COMEMORY_DATA_DIR", data_dir)
        // Tracking bumps access_count, which feeds ACT-R activation and
        // would reorder every query after the first.
        .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "search {query:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope: SearchEnvelope =
        serde_json::from_slice(&out.stdout).expect("search --json emits a parseable envelope");
    envelope.hits
}

#[test]
fn ranking_is_unchanged_for_every_recorded_query() {
    let fx = fixture();
    assert!(
        !fx.expected.is_empty(),
        "the fixture must carry at least one recorded query"
    );

    let dir = tempdir().unwrap();
    restore_corpus(dir.path(), &fx.corpus);

    for expected in &fx.expected {
        let actual = run_query(dir.path(), &expected.query);

        let want: Vec<&str> = expected.hits.iter().map(|h| h.memory_id.as_str()).collect();
        let got: Vec<&str> = actual.iter().map(|h| h.memory_id.as_str()).collect();
        assert_eq!(
            got, want,
            "hit ordering moved for query {:?} — the change was supposed to be \
             carry-through only (spec AC-22)",
            expected.query
        );

        for (want_hit, got_hit) in expected.hits.iter().zip(actual.iter()) {
            assert_eq!(
                want_hit.tier, got_hit.tier,
                "lexical ladder tier moved for {} on query {:?}",
                want_hit.memory_id, expected.query
            );
            assert_parts(
                &want_hit.score_parts,
                &got_hit.score_parts,
                &want_hit.memory_id,
                &expected.query,
            );
            // `score` duplicates `final_score`, which inherits the
            // time-dependent activation factor — so it is checked for
            // internal agreement, not against the aged fixture.
            assert!(
                (got_hit.score - got_hit.score_parts.final_score).abs() < PRODUCT_TOLERANCE,
                "score must mirror final_score for {} on query {:?}",
                want_hit.memory_id,
                expected.query
            );
        }
    }
}

/// Compare two score breakdowns.
///
/// `rrf`, `feedback`, `quality`, and `supersede` are pure functions of the
/// corpus and the query, so they must be bit-identical — a carry-through
/// change that disturbed any of them would be a real regression. `rank`
/// gets [`DRIFT_TOLERANCE`] for the PageRank convergence reason above.
///
/// `activation` is deliberately NOT compared against the fixture, and
/// neither is `final_score`, which is its product. ACT-R activation is
/// `ln(max(n,1)) - d*ln(days+1)`: a function of how long ago the memory was
/// touched, i.e. of *when the test runs*. It drifted 4e-3 within an hour of
/// recording this fixture and would drift ~36% over 90 days, so no fixed
/// tolerance can hold — a golden value for it would only ever measure the
/// calendar. [`assert_score_identity`] checks instead that `final_score`
/// still equals the product of the hit's own parts, which is
/// time-independent and is what a broken carry-through would violate.
fn assert_parts(want: &Parts, got: &Parts, id: &str, query: &str) {
    let ctx = format!("{id} on query {query:?}");
    assert_eq!(want.rrf, got.rrf, "rrf moved for {ctx}");
    assert_eq!(want.feedback, got.feedback, "feedback moved for {ctx}");
    assert_eq!(want.quality, got.quality, "quality moved for {ctx}");
    assert_eq!(want.supersede, got.supersede, "supersede moved for {ctx}");
    assert_close(want.rank, got.rank, &format!("rank for {ctx}"));
    assert_score_identity(got, &ctx);
}

/// `final_score` must remain the product of every factor beside it, checked
/// against the hit's OWN live values so it holds no matter how far the
/// fixture has aged. This is what breaks if a factor is dropped from, or
/// double-applied in, the rerank chain.
fn assert_score_identity(got: &Parts, ctx: &str) {
    let product = got.rrf * got.activation * got.feedback * got.quality * got.supersede * got.rank;
    let rel = (product - got.final_score).abs() / product.abs().max(1e-12);
    assert!(
        rel < PRODUCT_TOLERANCE,
        "final_score is no longer the product of its parts for {ctx}: {product} vs {} \
         (relative {rel:e})",
        got.final_score
    );
}

/// Relative-difference assertion using [`DRIFT_TOLERANCE`].
fn assert_close(want: f64, got: f64, what: &str) {
    let scale = want.abs().max(1e-12);
    let rel = (want - got).abs() / scale;
    assert!(
        rel < DRIFT_TOLERANCE,
        "{what} moved beyond drift tolerance: {want} -> {got} (relative {rel:e})"
    );
}
