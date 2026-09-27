#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Logout configuration recovery against real files and subprocess environments.

#[test]
fn logout_config_preserves_valid_env_and_recovers_from_invalid_env() {
    const CASE: &str = "COMEMORY_TEST_LOGOUT_ENV";
    if let Ok(case) = std::env::var(CASE) {
        let home = tempfile::tempdir().unwrap();
        let paths = crate::config::Paths::new(home.path());
        std::fs::write(paths.config_file(), "[invalid toml").unwrap();
        let cfg = super::logout_config(&paths);
        match case.as_str() {
            "valid" => assert!(!cfg.sync.push_on_save),
            "invalid" => assert_eq!(
                cfg.sync.push_on_save,
                crate::config::Config::defaults().sync.push_on_save
            ),
            other => panic!("unknown test case: {other}"),
        }
        return;
    }
    for case in ["valid", "invalid"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cli::auth::tests::logout_config_preserves_valid_env_and_recovers_from_invalid_env",
                "--nocapture",
            ])
            .env(CASE, case)
            .env("COMEMORY_SYNC_PUSH_ON_SAVE", "false")
            .env(
                "COMEMORY_SYNC_CODE_INDEX",
                if case == "valid" { "true" } else { "invalid" },
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{case}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }
}
