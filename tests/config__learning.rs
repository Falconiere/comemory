//! Mirror for `src/config/learning.rs` — reinforce / bandit defaults, env
//! overlay, validation, and `Paths::config_file` layout.

use comemory::config::{BanditConfig, Config, Paths, ReinforceConfig};

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
fn paths_config_file_is_data_dir_config_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(dir.path().to_path_buf());
    assert_eq!(paths.config_file(), dir.path().join("config.toml"));
}
