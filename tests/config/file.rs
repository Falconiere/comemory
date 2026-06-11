use comemory::config::file::{AutoReindexMode, Config, PruneConfig, RankConfig};

#[test]
fn defaults_match_spec() {
    let c = Config::defaults();
    assert_eq!(c.embeddings.memory_model, "nomic-embed-text-v1.5-Q");
    assert_eq!(c.embeddings.code_model, "jina-embeddings-v2-base-code-Q");
    assert!(matches!(c.indexing.auto_reindex, AutoReindexMode::Lazy));
    assert_eq!(c.indexing.auto_reindex_threshold_ms, 200);
    assert_eq!(c.retrieval.memory_threshold, 0.55);
    assert_eq!(c.retrieval.hybrid_weight, 0.65);
    assert_eq!(c.retrieval.top_k, 12);
    assert_eq!(c.prune.trash_retention_days, 30);
    assert!(
        (c.retrieval.rrf_k - 60.0).abs() < f32::EPSILON,
        "default rrf_k must be 60.0"
    );
}

#[test]
fn config_has_no_dead_knobs() {
    let cfg = comemory::config::Config::defaults();
    // Struct compiles without the removed field — this test pins the
    // serialized shape so a re-introduction is caught. (code_threshold,
    // removed in M2 for having no consumer, returned in M3 with the
    // code-search wiring and is no longer a dead knob.)
    let toml = toml::to_string(&cfg).expect("serialize defaults");
    assert!(!toml.contains("low_value_default_unused_since_days"));
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

// ── config.toml file-overlay tests ───────────────────────────────────────────

#[test]
fn prune_file_overlay_accepts_existing_fields() {
    // Regression: PartialPruneConfig initially carried only the M1 scoring
    // extensions (min_activation / min_feedback) under deny_unknown_fields,
    // so a valid `[prune] trash_retention_days = 60` in config.toml
    // hard-errored at startup. Every consumed PruneConfig field must be
    // overlayable.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[prune]\n\
         trash_retention_days = 60\n\
         low_value_default_below_quality = 4\n",
    )
    .expect("write config.toml");
    let cfg = Config::defaults()
        .with_file(&path)
        .expect("existing prune keys in [prune] must parse and apply");
    assert_eq!(cfg.prune.trash_retention_days, 60);
    assert_eq!(cfg.prune.low_value_default_below_quality, 4);
    // Untouched keys keep their defaults.
    assert_eq!(cfg.prune.min_activation, -2.0);
    assert_eq!(cfg.prune.min_feedback, 0.25);
}

#[test]
fn prune_overlay_rejects_legacy_unused_since_days_key() {
    // The legacy low_value_default_unused_since_days knob was removed in
    // M2 (zero consumers since M1), so a config.toml setting it errors
    // loudly (deny_unknown_fields) instead of silently no-opping.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[prune]\nlow_value_default_unused_since_days = 90\n")
        .expect("write config.toml");
    let err = Config::defaults()
        .with_file(&path)
        .expect_err("legacy key must be rejected");
    assert!(
        err.to_string()
            .contains("low_value_default_unused_since_days"),
        "error must name the rejected key, got: {err}"
    );
}

#[test]
fn file_overlay_invalid_values_are_an_error() {
    // Regression: the file overlay used to bypass validation entirely —
    // `[rank] decay = -1.0` in config.toml was silently accepted while
    // COMEMORY_RANK_DECAY=-1.0 hard-errored. Both layers now run the same
    // validate() pass, and the message names the offending field + env var.
    let cases: &[(&str, &str)] = &[
        ("[rank]\ndecay = -1.0\n", "rank.decay"),
        ("[rank]\nmmr_lambda = 2.0\n", "rank.mmr_lambda"),
        ("[rank]\nprior_clamp = [2.0, 0.5]\n", "rank.prior_clamp"),
        ("[prune]\nmin_feedback = 1.5\n", "prune.min_feedback"),
        (
            "[prune]\nlow_value_default_below_quality = 0\n",
            "prune.low_value_default_below_quality",
        ),
    ];
    let dir = tempfile::tempdir().expect("create temp dir");
    for (toml_body, field) in cases {
        let path = dir.path().join("config.toml");
        std::fs::write(&path, toml_body).expect("write config.toml");
        let err = Config::defaults()
            .with_file(&path)
            .expect_err(&format!("invalid {field} in config.toml must error"));
        let msg = err.to_string();
        assert!(
            msg.contains(field),
            "error must name the offending field '{field}', got: {msg}"
        );
    }
}

#[test]
fn learning_retention_days_defaults_to_90() {
    // Task 12: telemetry retention window for `comemory gc`. Aggregated
    // `feedback` counters are permanent; only raw event rows age out.
    assert_eq!(Config::defaults().prune.learning_retention_days, 90);
}

#[test]
fn learning_retention_days_file_overlay_applies() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[prune]\nlearning_retention_days = 7\n").expect("write config.toml");
    let cfg = Config::defaults()
        .with_file(&path)
        .expect("valid learning_retention_days must parse and apply");
    assert_eq!(cfg.prune.learning_retention_days, 7);
    // Untouched prune keys keep their defaults.
    assert_eq!(cfg.prune.trash_retention_days, 30);
}

#[test]
fn learning_retention_days_zero_in_file_is_an_error() {
    // 0 would make `gc` evict telemetry written microseconds ago,
    // including the rows `mine` needs; the floor is one day.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[prune]\nlearning_retention_days = 0\n").expect("write config.toml");
    let err = Config::defaults()
        .with_file(&path)
        .expect_err("learning_retention_days=0 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("prune.learning_retention_days"),
        "error must name the offending field, got: {msg}"
    );
}

#[test]
fn bm25_weights_default_and_file_overlay() {
    let cfg = Config::defaults();
    assert_eq!(cfg.retrieval.bm25_weights, (1.0, 3.0));

    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[retrieval]\nbm25_weights = [2.0, 1.0]\n").expect("write");
    let cfg = Config::defaults().with_file(&path).expect("load");
    assert_eq!(cfg.retrieval.bm25_weights, (2.0, 1.0));
}

#[test]
fn bm25_weights_rejects_negative_and_zero_pair() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[retrieval]\nbm25_weights = [0.0, 0.0]\n").expect("write");
    assert!(Config::defaults().with_file(&path).is_err());

    std::fs::write(&path, "[retrieval]\nbm25_weights = [-1.0, 3.0]\n").expect("write");
    assert!(Config::defaults().with_file(&path).is_err());
}

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
