//! Mirrors `src/config/file.rs` — Config file-overlay behavior (part 2):
//! retrieval section overlays, code knobs, tune grids, and rank/prune scoring.

use comemory::config::file::Config;

#[test]
fn retrieval_file_overlay_applies_tunable_keys() {
    // The [retrieval] section previously hard-errored under
    // deny_unknown_fields (no PartialRetrievalConfig existed); the four
    // M2-tunable keys are now overlayable, and absent keys keep defaults.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[retrieval]\n\
         rrf_k = 30.0\n\
         top_k = 8\n\
         memory_threshold = 0.4\n",
    )
    .expect("write config.toml");
    let cfg = Config::defaults()
        .with_file(&path)
        .expect("retrieval keys must parse and apply");
    assert!((cfg.retrieval.rrf_k - 30.0).abs() < f32::EPSILON);
    assert_eq!(cfg.retrieval.top_k, 8);
    assert!((cfg.retrieval.memory_threshold - 0.4).abs() < f32::EPSILON);
    // bm25_weights absent from the file → default retained.
    assert_eq!(cfg.retrieval.bm25_weights, (1.0, 3.0));
}

#[test]
fn retrieval_file_overlay_rejects_invalid_rrf_k() {
    // rrf_k validation moved from the env arm into validate() so the file
    // overlay enforces the same finite-positive invariant.
    let dir = tempfile::tempdir().expect("create temp dir");
    for bad in ["rrf_k = 0.0", "rrf_k = -1.0", "rrf_k = nan"] {
        let path = dir.path().join("config.toml");
        std::fs::write(&path, format!("[retrieval]\n{bad}\n")).expect("write config.toml");
        let err = Config::defaults()
            .with_file(&path)
            .expect_err(&format!("invalid '{bad}' in config.toml must error"));
        let msg = err.to_string();
        assert!(
            msg.contains("rrf_k"),
            "error must name the offending field for '{bad}', got: {msg}"
        );
    }
}

#[test]
fn file_overlay_prior_clamp_array_applies() {
    // Pins the serde representation of the (f64, f64) tuple: in TOML a
    // tuple is written as a two-element array.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[rank]\nprior_clamp = [0.6, 1.8]\n").expect("write config.toml");
    let cfg = Config::defaults()
        .with_file(&path)
        .expect("valid prior_clamp array must parse and apply");
    assert_eq!(cfg.rank.prior_clamp, (0.6, 1.8));
}

#[test]
fn file_overlay_unknown_rank_key_is_an_error() {
    // deny_unknown_fields applies inside [rank] too: a typo'd key must
    // hard-error at load time instead of being silently dropped.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[rank]\ndecai = 0.3\n").expect("write config.toml");
    let err = Config::defaults()
        .with_file(&path)
        .expect_err("unknown key inside [rank] must error");
    let msg = err.to_string();
    assert!(
        msg.contains("decai"),
        "error must name the unknown key, got: {msg}"
    );
}

// ── M3 code knobs + configurable constants ───────────────────────────────────

#[test]
fn code_knob_defaults() {
    let cfg = Config::defaults();
    assert!(
        (cfg.retrieval.code_threshold - 0.50).abs() < f32::EPSILON,
        "default code_threshold must be 0.50"
    );
    // Column order (symbol, snippet, path_tokens) — mirrors the previously
    // hardcoded `bm25(code_fts, 0.0, 2.0, 1.0, 1.5)`.
    assert_eq!(cfg.retrieval.code_bm25_weights, (2.0, 1.0, 1.5));
    assert_eq!(cfg.rank.near_dup_hamming, 8);
    assert_eq!(cfg.prune.superseded_grace_days, 7);
}

#[test]
fn tune_grid_defaults() {
    // The M1 3×3×3×3 grid, now configuration instead of hardcoded loops.
    let cfg = Config::defaults();
    assert_eq!(cfg.tune.rrf_k_grid, vec![20.0f32, 60.0, 100.0]);
    assert_eq!(cfg.tune.decay_grid, vec![0.3f64, 0.5, 0.8]);
    assert_eq!(cfg.tune.mmr_lambda_grid, vec![0.5f64, 0.7, 0.9]);
    assert_eq!(
        cfg.tune.bm25_grid,
        vec![(1.0f32, 3.0f32), (1.0, 1.0), (2.0, 1.0)]
    );
}

#[test]
fn code_knob_file_overlays_apply() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[retrieval]\n\
         code_threshold = 0.6\n\
         code_bm25_weights = [3.0, 1.0, 2.0]\n\
         [rank]\n\
         near_dup_hamming = 4\n\
         [prune]\n\
         superseded_grace_days = 14\n",
    )
    .expect("write config.toml");
    let cfg = Config::defaults()
        .with_file(&path)
        .expect("code knob overlays must parse and apply");
    assert!((cfg.retrieval.code_threshold - 0.6).abs() < f32::EPSILON);
    assert_eq!(cfg.retrieval.code_bm25_weights, (3.0, 1.0, 2.0));
    assert_eq!(cfg.rank.near_dup_hamming, 4);
    assert_eq!(cfg.prune.superseded_grace_days, 14);
}

#[test]
fn tune_grid_file_overlay_applies() {
    // Grids are file-only (no COMEMORY_TUNE_* env vars: a four-list env
    // value is unreadable). Absent grid keys keep their defaults.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[tune]\n\
         rrf_k_grid = [60.0]\n\
         decay_grid = [0.5, 0.7]\n\
         bm25_grid = [[2.0, 1.0]]\n",
    )
    .expect("write config.toml");
    let cfg = Config::defaults()
        .with_file(&path)
        .expect("tune grid overlays must parse and apply");
    assert_eq!(cfg.tune.rrf_k_grid, vec![60.0f32]);
    assert_eq!(cfg.tune.decay_grid, vec![0.5f64, 0.7]);
    assert_eq!(cfg.tune.bm25_grid, vec![(2.0f32, 1.0f32)]);
    // mmr_lambda_grid absent from the file → default retained.
    assert_eq!(cfg.tune.mmr_lambda_grid, vec![0.5f64, 0.7, 0.9]);
}

#[test]
fn tune_overlay_unknown_key_is_an_error() {
    // deny_unknown_fields applies inside [tune] too.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[tune]\nrrf_grid = [60.0]\n").expect("write config.toml");
    let err = Config::defaults()
        .with_file(&path)
        .expect_err("unknown key inside [tune] must error");
    assert!(
        err.to_string().contains("rrf_grid"),
        "error must name the unknown key, got: {err}"
    );
}

#[test]
fn rank_and_prune_file_overlay_applies_scoring_knobs() {
    // The [rank] section and the M1 prune scoring extensions are settable
    // from config.toml; absent keys keep defaults.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[rank]\n\
         decay = 0.8\n\
         mmr_lambda = 0.4\n\
         [prune]\n\
         min_activation = -3.0\n\
         min_feedback = 0.1\n",
    )
    .expect("write config.toml");
    let cfg = Config::defaults()
        .with_file(&path)
        .expect("rank + prune scoring keys must parse and apply");
    assert!((cfg.rank.decay - 0.8).abs() < f64::EPSILON);
    assert!((cfg.rank.mmr_lambda - 0.4).abs() < f64::EPSILON);
    // prior_clamp absent from the file → default retained.
    assert_eq!(cfg.rank.prior_clamp, (0.5, 2.0));
    assert!((cfg.prune.min_activation - (-3.0)).abs() < f64::EPSILON);
    assert!((cfg.prune.min_feedback - 0.1).abs() < f64::EPSILON);
}
