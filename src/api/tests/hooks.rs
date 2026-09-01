#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/hooks.rs`. Real filesystem writes under a real
//! `.git/hooks/` directory (same fixture shape `tests/api__install_hooks.rs`
//! uses) and a real `config.toml` round-trip through the typed `Config`
//! loader — no mocks (Binding Rule 9). `comemory hooks`'s subprocess-level
//! behavior (AC-35, AC-36) is covered in `tests/cli__hooks.rs`; the HTTP
//! route (including the read-only gate, AC-33b) lives in
//! `tests/serve__routes__hooks.rs`. This file exercises `api::hooks::run`
//! directly.

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};

fn fake_repo(home: &std::path::Path) -> std::path::PathBuf {
    let repo = home.join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("fake .git dir");
    repo
}

fn request(
    repo: &std::path::Path,
    enable: Option<&str>,
    disable: Option<&str>,
) -> api::hooks::Request {
    api::hooks::Request {
        repo: Some(repo.display().to_string()),
        enable: enable.map(str::to_string),
        disable: disable.map(str::to_string),
    }
}

#[test]
fn fresh_repo_reports_all_four_rows_with_git_hooks_uninstalled_and_reinforce_enabled() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = fake_repo(home.path());
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let resp = api::hooks::run(&mut ctx, request(&repo, None, None)).expect("hooks run");

    assert_eq!(resp.hooks.len(), 4);
    let names: Vec<&str> = resp.hooks.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "post-commit",
            "post-merge",
            "post-checkout",
            "search-edit-reinforcement",
        ]
    );
    for row in &resp.hooks[..3] {
        assert!(!row.installed, "{} must start uninstalled", row.name);
        assert_eq!(row.source, "git");
    }
    let reinforce = &resp.hooks[3];
    assert!(reinforce.installed, "reinforce defaults to enabled");
    assert_eq!(reinforce.source, "config");
}

#[test]
fn after_install_hooks_all_three_git_rows_report_installed() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = fake_repo(home.path());
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    api::install_hooks::run(
        &mut ctx,
        api::install_hooks::Request {
            repo: repo.display().to_string(),
            force: false,
        },
    )
    .expect("install-hooks run");

    let resp = api::hooks::run(&mut ctx, request(&repo, None, None)).expect("hooks run");

    for row in &resp.hooks[..3] {
        assert!(row.installed, "{} must be installed", row.name);
    }
}

#[test]
fn disable_one_git_hook_leaves_the_other_two_files_byte_identical() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = fake_repo(home.path());
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    api::install_hooks::run(
        &mut ctx,
        api::install_hooks::Request {
            repo: repo.display().to_string(),
            force: false,
        },
    )
    .expect("install-hooks run");
    let hooks_dir = repo.join(".git").join("hooks");
    let commit_before = std::fs::read(hooks_dir.join("post-commit")).expect("read post-commit");
    let merge_before = std::fs::read(hooks_dir.join("post-merge")).expect("read post-merge");

    let resp = api::hooks::run(&mut ctx, request(&repo, None, Some("post-checkout")))
        .expect("disable run");

    let by_name: std::collections::HashMap<_, _> = resp
        .hooks
        .iter()
        .map(|h| (h.name.as_str(), h.installed))
        .collect();
    assert!(!by_name["post-checkout"]);
    assert!(by_name["post-commit"]);
    assert!(by_name["post-merge"]);
    assert!(!hooks_dir.join("post-checkout").exists());
    assert_eq!(
        std::fs::read(hooks_dir.join("post-commit")).expect("read post-commit"),
        commit_before,
        "post-commit must be byte-identical after disabling a different hook"
    );
    assert_eq!(
        std::fs::read(hooks_dir.join("post-merge")).expect("read post-merge"),
        merge_before,
        "post-merge must be byte-identical after disabling a different hook"
    );
}

#[test]
fn disable_is_idempotent_on_an_already_missing_hook() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = fake_repo(home.path());
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let resp = api::hooks::run(&mut ctx, request(&repo, None, Some("post-commit")))
        .expect("disable on a never-installed hook must not error");
    assert!(!resp.hooks[0].installed);
}

#[test]
fn enable_reinstalls_a_previously_disabled_git_hook() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = fake_repo(home.path());
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    api::install_hooks::run(
        &mut ctx,
        api::install_hooks::Request {
            repo: repo.display().to_string(),
            force: false,
        },
    )
    .expect("install-hooks run");
    api::hooks::run(&mut ctx, request(&repo, None, Some("post-merge"))).expect("disable run");

    let resp =
        api::hooks::run(&mut ctx, request(&repo, Some("post-merge"), None)).expect("enable run");

    assert!(resp.hooks[1].installed, "post-merge must be reinstalled");
    let body = std::fs::read_to_string(repo.join(".git").join("hooks").join("post-merge"))
        .expect("read post-merge");
    assert!(body.contains("comemory index-code"));
}

#[test]
fn unknown_hook_name_is_rejected_and_writes_nothing() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = fake_repo(home.path());
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let err = api::hooks::run(&mut ctx, request(&repo, Some("pre-push"), None))
        .expect_err("unknown hook must be rejected");
    assert!(err.to_string().contains("pre-push"));
    assert!(!repo.join(".git").join("hooks").join("pre-push").exists());
}

#[test]
fn reinforce_toggle_round_trips_through_config_toml() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = fake_repo(home.path());
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let resp = api::hooks::run(
        &mut ctx,
        request(&repo, None, Some(api::hooks::REINFORCE_HOOK)),
    )
    .expect("disable reinforce");
    assert!(!resp.hooks[3].installed);

    // Round-trips through the real typed config loader, not just raw TOML —
    // proves the toggle doesn't break every other command's config load.
    let reloaded = Config::defaults()
        .with_file(&paths.config_file())
        .expect("config.toml must still parse after the toggle");
    assert!(!reloaded.reinforce.enabled);

    let mut ctx2 = Ctx::lazy(&paths, &reloaded);
    let resp2 = api::hooks::run(
        &mut ctx2,
        request(&repo, Some(api::hooks::REINFORCE_HOOK), None),
    )
    .expect("re-enable reinforce");
    assert!(resp2.hooks[3].installed);

    let reloaded2 = Config::defaults()
        .with_file(&paths.config_file())
        .expect("config.toml must still parse after re-enabling");
    assert!(reloaded2.reinforce.enabled);
}

#[test]
fn run_never_creates_the_database() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = fake_repo(home.path());
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    api::hooks::run(&mut ctx, request(&repo, None, None)).expect("hooks run");

    assert!(
        !paths.db_path().exists(),
        "listing hook state must not create and migrate a database"
    );
}
