#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror for `src/config/learning.rs` — tune grids/samples, reinforce /
//! bandit defaults, env + file overlay, validation, and
//! `Paths::config_file` layout.

use comemory::config::{
    BanditConfig, Config, ObservationsConfig, Paths, ReinforceConfig, TuneConfig,
};

#[test]
fn reinforce_default_search_edit_days_is_seven() {
    assert_eq!(ReinforceConfig::default().search_edit_days, 7);
    assert_eq!(Config::defaults().reinforce.search_edit_days, 7);
}

#[test]
fn bandit_default_enabled_is_true() {
    assert!(BanditConfig::default().enabled);
    assert!(Config::defaults().bandit.enabled);
}

#[test]
fn env_search_edit_days_override_applies() {
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race.
    unsafe { std::env::set_var("COMEMORY_REINFORCE_SEARCH_EDIT_DAYS", "3") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process.
    unsafe { std::env::remove_var("COMEMORY_REINFORCE_SEARCH_EDIT_DAYS") };
    let cfg = result.expect("valid override must succeed");
    assert_eq!(cfg.reinforce.search_edit_days, 3);
}

#[test]
fn env_search_edit_days_zero_fails_validate() {
    // SAFETY: nextest runs each #[test] in its own process.
    unsafe { std::env::set_var("COMEMORY_REINFORCE_SEARCH_EDIT_DAYS", "0") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process.
    unsafe { std::env::remove_var("COMEMORY_REINFORCE_SEARCH_EDIT_DAYS") };
    let err = result.expect_err("0 must fail validate");
    let msg = err.to_string();
    assert!(
        msg.contains("search_edit_days") || msg.contains("COMEMORY_REINFORCE_SEARCH_EDIT_DAYS"),
        "error must name the knob, got: {msg}"
    );
}

#[test]
fn tune_defaults_carry_the_graph_pools_and_sample_budget() {
    // The two graph pools widen the legacy 3^4 grid to 3^6 = 729 points,
    // of which a run samples 64 by default.
    for t in [TuneConfig::default(), Config::defaults().tune] {
        assert_eq!(t.graph_hops_grid, vec![0u32, 1, 2]);
        assert_eq!(t.graph_seeds_grid, vec![4usize, 8, 16]);
        assert_eq!(t.samples, 64);
    }
}

#[test]
fn file_overlay_replaces_the_graph_pools_and_samples() {
    // Absent keys keep their defaults; present ones replace wholesale.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[tune]\n\
         graph_hops_grid = [0, 2]\n\
         samples = 0\n",
    )
    .expect("write config.toml");
    let cfg = Config::defaults()
        .with_file(&path)
        .expect("overlay must load");
    assert_eq!(cfg.tune.graph_hops_grid, vec![0u32, 2]);
    assert_eq!(cfg.tune.samples, 0, "0 selects the exhaustive grid");
    assert_eq!(
        cfg.tune.graph_seeds_grid,
        vec![4usize, 8, 16],
        "an absent key must leave the default pool untouched"
    );
    assert_eq!(cfg.tune.decay_grid, TuneConfig::default().decay_grid);
}

#[test]
fn paths_config_file_is_data_dir_config_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(dir.path().to_path_buf());
    assert_eq!(paths.config_file(), dir.path().join("config.toml"));
}

#[test]
fn observation_capture_is_off_by_default_and_bounded() {
    let defaults = ObservationsConfig::default();
    assert!(
        !defaults.enabled,
        "capture stores passage snapshots and must be opted into, never inherited"
    );
    assert_eq!(defaults.max_text_bytes, 4096);
    assert_eq!(defaults.max_candidates, 100);
    let cfg = Config::defaults();
    assert!(!cfg.observations.enabled);
    assert_eq!(cfg.observations.max_text_bytes, 4096);
    assert_eq!(cfg.observations.max_candidates, 100);
}

#[test]
fn observations_env_overrides_apply() {
    // SAFETY: nextest runs each #[test] in its own process.
    unsafe {
        std::env::set_var("COMEMORY_OBSERVATIONS_ENABLED", "1");
        std::env::set_var("COMEMORY_OBSERVATIONS_MAX_TEXT_BYTES", "512");
        std::env::set_var("COMEMORY_OBSERVATIONS_MAX_CANDIDATES", "7");
    }
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process.
    unsafe {
        std::env::remove_var("COMEMORY_OBSERVATIONS_ENABLED");
        std::env::remove_var("COMEMORY_OBSERVATIONS_MAX_TEXT_BYTES");
        std::env::remove_var("COMEMORY_OBSERVATIONS_MAX_CANDIDATES");
    }
    let cfg = result.expect("valid overrides must succeed");
    assert!(cfg.observations.enabled);
    assert_eq!(cfg.observations.max_text_bytes, 512);
    assert_eq!(cfg.observations.max_candidates, 7);
}

#[test]
fn a_zero_observation_bound_fails_validate_naming_the_knob() {
    for (var, knob) in [
        ("COMEMORY_OBSERVATIONS_MAX_TEXT_BYTES", "max_text_bytes"),
        ("COMEMORY_OBSERVATIONS_MAX_CANDIDATES", "max_candidates"),
    ] {
        // SAFETY: nextest runs each #[test] in its own process.
        unsafe { std::env::set_var(var, "0") };
        let result = Config::defaults().with_env();
        // SAFETY: nextest runs each #[test] in its own process.
        unsafe { std::env::remove_var(var) };
        let msg = result
            .err()
            .map_or_else(|| panic!("{var}=0 must fail validate"), |e| e.to_string());
        assert!(
            msg.contains(knob) || msg.contains(var),
            "error must name the knob, got: {msg}"
        );
    }
}

#[test]
fn a_file_overlay_sets_the_observation_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[observations]\nenabled = true\nmax_text_bytes = 128\nmax_candidates = 3\n",
    )
    .expect("write config");
    let cfg = Config::defaults().with_file(&path).expect("overlay loads");
    assert!(cfg.observations.enabled);
    assert_eq!(cfg.observations.max_text_bytes, 128);
    assert_eq!(cfg.observations.max_candidates, 3);
}
