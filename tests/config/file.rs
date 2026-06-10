use comemory::config::file::{AutoReindexMode, Config, PruneConfig, RankConfig};

#[test]
fn defaults_match_spec() {
    let c = Config::defaults();
    assert_eq!(c.embeddings.memory_model, "nomic-embed-text-v1.5-Q");
    assert_eq!(c.embeddings.code_model, "jina-embeddings-v2-base-code-Q");
    assert!(matches!(c.indexing.auto_reindex, AutoReindexMode::Lazy));
    assert_eq!(c.indexing.auto_reindex_threshold_ms, 200);
    assert_eq!(c.retrieval.memory_threshold, 0.55);
    assert_eq!(c.retrieval.code_threshold, 0.50);
    assert_eq!(c.retrieval.hybrid_weight, 0.65);
    assert_eq!(c.retrieval.top_k, 12);
    assert_eq!(c.prune.trash_retention_days, 30);
    assert!(
        (c.retrieval.rrf_k - 60.0).abs() < f32::EPSILON,
        "default rrf_k must be 60.0"
    );
}

#[test]
fn env_overrides_apply() {
    // SAFETY: serial within this test; env vars scoped and removed after read.
    std::env::set_var("COMEMORY_INDEXING_AUTO_REINDEX", "hook");
    std::env::set_var("COMEMORY_RETRIEVAL_TOP_K", "20");
    let c = Config::defaults().with_env().unwrap();
    std::env::remove_var("COMEMORY_INDEXING_AUTO_REINDEX");
    std::env::remove_var("COMEMORY_RETRIEVAL_TOP_K");
    assert!(matches!(c.indexing.auto_reindex, AutoReindexMode::Hook));
    assert_eq!(c.retrieval.top_k, 20);
}

#[test]
fn env_rrf_k_override_applies() {
    // Regression for C3: the CLI must read rrf_k through Config instead of
    // hardcoding 60.0. Verify the env var path drops a valid override into
    // the retrieval config so callers (search, context) can consume it.
    //
    // This test exercises only one env var and reads through `with_env()`
    // immediately, mirroring the pre-existing env-var tests in this file.
    // Run with `--test-threads=1` to avoid races with other env-mutating
    // tests (a pre-existing constraint of this test binary).
    std::env::set_var("COMEMORY_RETRIEVAL_RRF_K", "42.0");
    let c = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_RETRIEVAL_RRF_K");
    let cfg = c.expect("rrf_k override must succeed");
    assert!((cfg.retrieval.rrf_k - 42.0).abs() < 1e-6);
}

#[test]
fn env_invalid_rrf_k_returns_err() {
    // Regression for G3: NaN / inf / non-positive rrf_k values must surface
    // as `Err` rather than silently falling back to the default. They would
    // cause `1 / (k + rank)` to collapse to NaN or pin every score to the
    // same bucket, silently destroying the ranking — so a typo must abort
    // at startup, matching the style used by `top_k` / `memory_threshold`.
    //
    // Run with `--test-threads=1` (already required by this binary) so env
    // vars don't race with the other Config tests.
    for bad in ["nan", "NaN", "inf", "-inf", "0", "-1"] {
        std::env::set_var("COMEMORY_RETRIEVAL_RRF_K", bad);
        let result = Config::defaults().with_env();
        std::env::remove_var("COMEMORY_RETRIEVAL_RRF_K");
        let err = result.expect_err(&format!("'{bad}' must error"));
        let msg = err.to_string();
        assert!(
            msg.contains("COMEMORY_RETRIEVAL_RRF_K"),
            "error must name the offending var for '{bad}', got: {msg}"
        );
    }
}

#[test]
fn env_rejects_invalid_top_k() {
    // Non-numeric top_k must surface as Err instead of silently keeping the
    // default; otherwise typos go unnoticed until retrieval misbehaves.
    std::env::set_var("COMEMORY_RETRIEVAL_TOP_K", "not-a-number");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_RETRIEVAL_TOP_K");
    let err = result.expect_err("invalid top_k must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RETRIEVAL_TOP_K"),
        "error message should name the offending var, got: {msg}"
    );
}

#[test]
fn env_rejects_invalid_auto_reindex() {
    // Unknown mode (typo of "hook") must error, not fall through to Lazy.
    std::env::set_var("COMEMORY_INDEXING_AUTO_REINDEX", "hooks");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_INDEXING_AUTO_REINDEX");
    let err = result.expect_err("unknown auto_reindex must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_INDEXING_AUTO_REINDEX"),
        "error message should name the offending var, got: {msg}"
    );
}

#[test]
fn env_rejects_invalid_auto_sync() {
    // Boolean parser only accepts true|1|yes|on / false|0|no|off; anything
    // else (e.g. "maybe") must error.
    std::env::set_var("COMEMORY_GIT_AUTO_SYNC", "maybe");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_GIT_AUTO_SYNC");
    let err = result.expect_err("unknown auto_sync must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_GIT_AUTO_SYNC"),
        "error message should name the offending var, got: {msg}"
    );
}

#[test]
fn toml_round_trip() {
    let c = Config::defaults();
    let s = toml::to_string(&c).expect("serialize defaults to toml");
    let back: Config = toml::from_str(&s).expect("deserialize defaults from toml");
    assert_eq!(back.retrieval.top_k, c.retrieval.top_k);
}

#[test]
fn default_vector_dims_match_ddl() {
    // The authoritative dim lives in src/store/sql/0002_v2_tables.sql:
    // `memory_vec FLOAT[1024]` and `code_vec FLOAT[768]`. Config defaults
    // mirror those literals for `comemory doctor` reporting; they cannot
    // be overridden via env (a divergent env value would just surface as
    // `VecDimMismatch` at first insert).
    let c = Config::defaults();
    assert_eq!(c.retrieval.memory_vector_dim, 1024);
    assert_eq!(c.retrieval.code_vector_dim, 768);
}

#[test]
fn default_embed_hint_is_unset() {
    // Spec §7: COMEMORY_EMBED_HINT is purely informational and starts unset.
    let c = Config::defaults();
    assert!(c.embed_hint.is_none());
}

#[test]
fn env_vector_dim_is_ignored() {
    // Regression for PR #3 review thread: COMEMORY_VECTOR_DIM used to feed
    // `with_env` and silently overwrite `retrieval.memory_vector_dim`,
    // creating a footgun where the config value disagreed with the
    // hardcoded vec0 DDL. `with_env` must leave the field on its default
    // even when the env var is set.
    std::env::set_var("COMEMORY_VECTOR_DIM", "512");
    let c = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_VECTOR_DIM");
    let cfg = c.expect("with_env must succeed even with COMEMORY_VECTOR_DIM set");
    assert_eq!(
        cfg.retrieval.memory_vector_dim, 1024,
        "COMEMORY_VECTOR_DIM must not override the DDL-locked dim"
    );
}

#[test]
fn env_code_vector_dim_is_ignored() {
    std::env::set_var("COMEMORY_CODE_VECTOR_DIM", "384");
    let c = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_CODE_VECTOR_DIM");
    let cfg = c.expect("with_env must succeed even with COMEMORY_CODE_VECTOR_DIM set");
    assert_eq!(
        cfg.retrieval.code_vector_dim, 768,
        "COMEMORY_CODE_VECTOR_DIM must not override the DDL-locked dim"
    );
}

#[test]
fn env_embed_hint_overrides_apply() {
    // Doctor surfaces this verbatim so callers know which embedder filled
    // the vectors. It is opaque to comemory itself.
    std::env::set_var("COMEMORY_EMBED_HINT", "ollama:nomic-embed-text");
    let c = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_EMBED_HINT");
    let cfg = c.expect("embed hint override must succeed");
    assert_eq!(cfg.embed_hint.as_deref(), Some("ollama:nomic-embed-text"));
}

// ── RankConfig + PruneConfig extension tests ─────────────────────────────────

#[test]
fn rank_config_is_accessible() {
    // RankConfig and PruneConfig are public items re-exported from config::file.
    // Verify the types exist and their fields compile.
    let _r: RankConfig = Config::defaults().rank;
    let _p: PruneConfig = Config::defaults().prune;
}

#[test]
fn rank_defaults() {
    let cfg = Config::defaults();
    assert_eq!(cfg.rank.decay, 0.5);
    assert_eq!(cfg.rank.prior_clamp, (0.5, 2.0));
    assert_eq!(cfg.rank.mmr_lambda, 0.7);
    assert_eq!(cfg.prune.min_activation, -2.0);
    assert_eq!(cfg.prune.min_feedback, 0.25);
}

#[test]
fn rank_env_overrides() {
    // All five new rank/prune env vars must be picked up by with_env().
    std::env::set_var("COMEMORY_RANK_DECAY", "0.7");
    std::env::set_var("COMEMORY_RANK_PRIOR_CLAMP", "0.6,1.8");
    std::env::set_var("COMEMORY_RANK_MMR_LAMBDA", "0.5");
    std::env::set_var("COMEMORY_PRUNE_MIN_ACTIVATION", "-1.5");
    std::env::set_var("COMEMORY_PRUNE_MIN_FEEDBACK", "0.3");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_RANK_DECAY");
    std::env::remove_var("COMEMORY_RANK_PRIOR_CLAMP");
    std::env::remove_var("COMEMORY_RANK_MMR_LAMBDA");
    std::env::remove_var("COMEMORY_PRUNE_MIN_ACTIVATION");
    std::env::remove_var("COMEMORY_PRUNE_MIN_FEEDBACK");
    let cfg = result.expect("all valid rank/prune env vars must succeed");
    assert!((cfg.rank.decay - 0.7).abs() < f64::EPSILON);
    assert_eq!(cfg.rank.prior_clamp, (0.6, 1.8));
    assert!((cfg.rank.mmr_lambda - 0.5).abs() < f64::EPSILON);
    assert!((cfg.prune.min_activation - (-1.5)).abs() < f64::EPSILON);
    assert!((cfg.prune.min_feedback - 0.3).abs() < f64::EPSILON);
}

#[test]
fn bad_clamp_lo_gt_hi_is_an_error() {
    // lo > hi is an invalid configuration: RRF rank fusion requires a valid
    // [lo, hi] interval. This must surface as Err rather than silent no-op.
    std::env::set_var("COMEMORY_RANK_PRIOR_CLAMP", "2.0,0.5");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_RANK_PRIOR_CLAMP");
    let err = result.expect_err("lo > hi prior_clamp must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RANK_PRIOR_CLAMP"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn bad_decay_negative_is_an_error() {
    // Negative decay violates the ACT-R formula (ln(n) - d*ln(days+1)):
    // a negative d would invert recency weighting. Must surface as Err.
    std::env::set_var("COMEMORY_RANK_DECAY", "-1.0");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_RANK_DECAY");
    let err = result.expect_err("negative decay must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RANK_DECAY"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn bad_mmr_lambda_out_of_range_is_an_error() {
    // MMR lambda must be in [0, 1]. A value > 1 has no defined meaning in
    // the relevance-vs-diversity trade-off and must be rejected.
    std::env::set_var("COMEMORY_RANK_MMR_LAMBDA", "2.0");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_RANK_MMR_LAMBDA");
    let err = result.expect_err("lambda > 1 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RANK_MMR_LAMBDA"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn bad_min_feedback_out_of_range_is_an_error() {
    // min_feedback is a beta-feedback value in [0, 1]. A value > 1 means no
    // memory is ever prune-eligible, which is a configuration error.
    std::env::set_var("COMEMORY_PRUNE_MIN_FEEDBACK", "1.5");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_PRUNE_MIN_FEEDBACK");
    let err = result.expect_err("min_feedback > 1 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_PRUNE_MIN_FEEDBACK"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn env_prune_below_quality_override_applies() {
    // COMEMORY_PRUNE_BELOW_QUALITY newly wired to the existing field
    // low_value_default_below_quality. Valid range 1..=5.
    std::env::set_var("COMEMORY_PRUNE_BELOW_QUALITY", "3");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_PRUNE_BELOW_QUALITY");
    let cfg = result.expect("valid below_quality override must succeed");
    assert_eq!(cfg.prune.low_value_default_below_quality, 3);
}

#[test]
fn bad_prune_below_quality_out_of_range_is_an_error() {
    // Quality 0 is below the minimum allowed value of 1.
    std::env::set_var("COMEMORY_PRUNE_BELOW_QUALITY", "0");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_PRUNE_BELOW_QUALITY");
    let err = result.expect_err("below_quality=0 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_PRUNE_BELOW_QUALITY"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn env_prune_unused_since_days_override_applies() {
    // COMEMORY_PRUNE_UNUSED_SINCE_DAYS newly wired to the existing field
    // low_value_default_unused_since_days.
    std::env::set_var("COMEMORY_PRUNE_UNUSED_SINCE_DAYS", "90");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_PRUNE_UNUSED_SINCE_DAYS");
    let cfg = result.expect("valid unused_since_days override must succeed");
    assert_eq!(cfg.prune.low_value_default_unused_since_days, 90);
}

#[test]
fn rank_toml_round_trip() {
    // RankConfig fields must survive a serde_json/toml round-trip so
    // config.toml writers can inspect their settings.
    let c = Config::defaults();
    let s = toml::to_string(&c).expect("serialize defaults to toml");
    let back: Config = toml::from_str(&s).expect("deserialize defaults from toml");
    assert!((back.rank.decay - c.rank.decay).abs() < f64::EPSILON);
    assert_eq!(back.rank.prior_clamp, c.rank.prior_clamp);
    assert!((back.rank.mmr_lambda - c.rank.mmr_lambda).abs() < f64::EPSILON);
}

#[test]
fn bad_clamp_lo_zero_is_an_error() {
    // lo must be > 0 per spec — a zero lower bound would collapse the clamp
    // interval to a single point and defeat multiplier bounding.
    std::env::set_var("COMEMORY_RANK_PRIOR_CLAMP", "0.0,1.5");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_RANK_PRIOR_CLAMP");
    let err = result.expect_err("clamp lo=0 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RANK_PRIOR_CLAMP"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn bad_clamp_wrong_arity_is_an_error() {
    // Only "lo,hi" (two values) is accepted. A bare scalar or triple is invalid.
    for bad in ["0.5", "0.5,1.0,1.5"] {
        std::env::set_var("COMEMORY_RANK_PRIOR_CLAMP", bad);
        let result = Config::defaults().with_env();
        std::env::remove_var("COMEMORY_RANK_PRIOR_CLAMP");
        let err = result.expect_err(&format!("bad clamp arity '{bad}' must error"));
        let msg = err.to_string();
        assert!(
            msg.contains("COMEMORY_RANK_PRIOR_CLAMP"),
            "error must name the offending var for '{bad}', got: {msg}"
        );
    }
}

#[test]
fn prune_file_overlay_accepts_existing_fields() {
    // Regression: PartialPruneConfig initially carried only the M1 scoring
    // extensions (min_activation / min_feedback) under deny_unknown_fields,
    // so a valid `[prune] trash_retention_days = 60` in config.toml
    // hard-errored at startup. Every PruneConfig field must be overlayable.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[prune]\n\
         trash_retention_days = 60\n\
         low_value_default_unused_since_days = 90\n\
         low_value_default_below_quality = 4\n",
    )
    .expect("write config.toml");
    let cfg = Config::defaults()
        .with_file(&path)
        .expect("existing prune keys in [prune] must parse and apply");
    assert_eq!(cfg.prune.trash_retention_days, 60);
    assert_eq!(cfg.prune.low_value_default_unused_since_days, 90);
    assert_eq!(cfg.prune.low_value_default_below_quality, 4);
    // Untouched keys keep their defaults.
    assert_eq!(cfg.prune.min_activation, -2.0);
    assert_eq!(cfg.prune.min_feedback, 0.25);
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
