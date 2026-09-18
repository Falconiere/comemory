//! `domains::integrations::install` host validation and dry-run reporting against a real
//! temporary data directory.
use super::{HOSTS, Request, config_dir, run, validate_host};
use crate::config::{Config, Paths};
use crate::utilities::context::Ctx;

/// Build a `Ctx` over a real temporary data directory.
fn ctx_for(dir: &std::path::Path) -> (Paths, Config) {
    (Paths::new(dir.to_path_buf()), Config::defaults())
}

#[test]
fn validate_host_accepts_the_known_hosts_and_names_the_rest() {
    for host in HOSTS {
        assert_eq!(validate_host(host).unwrap(), *host);
    }
    let err = validate_host("emacs").unwrap_err().to_string();
    assert!(
        err.contains("emacs"),
        "error should name the bad host: {err}"
    );
    assert!(
        err.contains("claude"),
        "error should list known hosts: {err}"
    );
}

#[test]
fn dry_run_reports_the_bundle_under_the_ctx_data_dir_without_writing() {
    let temp = tempfile::tempdir().unwrap();
    let (paths, cfg) = ctx_for(temp.path());
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = run(
        &mut ctx,
        Request {
            host: "claude".to_string(),
            dry_run: true,
            config_dir: Some(temp.path().join("claude-config")),
        },
    )
    .unwrap();

    assert!(resp.dry_run);
    assert!(!resp.installed);
    assert_eq!(resp.host, "claude");
    assert_eq!(resp.plugin, "comemory@comemory");
    assert!(
        resp.bundle.starts_with(temp.path()),
        "bundle must live under the Ctx data dir, got {}",
        resp.bundle.display()
    );
    assert!(
        resp.bundle.ends_with(env!("CARGO_PKG_VERSION")),
        "bundle path is version-scoped, got {}",
        resp.bundle.display()
    );
    // A preview writes nothing: neither the bundle nor the config dir appears.
    assert!(!resp.bundle.exists());
    assert!(!temp.path().join("claude-config").exists());
    // `comemory.db` is never created — install is conn-free.
    assert!(!temp.path().join("comemory.db").exists());
}

#[test]
fn run_rejects_an_unknown_host_before_touching_the_filesystem() {
    let temp = tempfile::tempdir().unwrap();
    let (paths, cfg) = ctx_for(temp.path());
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let err = run(
        &mut ctx,
        Request {
            host: "emacs".to_string(),
            dry_run: false,
            config_dir: Some(temp.path().join("nope")),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("emacs"));
    assert!(!temp.path().join("nope").exists());
}

#[test]
fn config_dir_prefers_an_explicit_override_and_absolutizes_it() {
    let temp = tempfile::tempdir().unwrap();
    let explicit = temp.path().join("explicit");
    let resolved = config_dir("claude", Some(explicit.clone())).unwrap();
    assert!(resolved.is_absolute());
    assert!(resolved.ends_with("explicit"));
}

#[test]
fn the_installed_marker_is_per_host_not_per_shared_bundle() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path();

    // Nothing installed yet.
    for host in HOSTS {
        assert!(!crate::domains::integrations::install::is_installed(
            data, host
        ));
    }

    // Simulate one host's successful install by writing only its marker.
    // The bundle tree itself is SHARED across hosts, so a detector keyed on
    // the bundle would now wrongly report every host as installed.
    let marker = crate::domains::integrations::install::marker_path(data, "claude");
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(&marker, env!("CARGO_PKG_VERSION")).unwrap();

    assert!(crate::domains::integrations::install::is_installed(
        data, "claude"
    ));
    assert!(
        !crate::domains::integrations::install::is_installed(data, "codex"),
        "registering claude must not make codex look installed"
    );
}

#[test]
fn a_dry_run_writes_no_installed_marker() {
    let temp = tempfile::tempdir().unwrap();
    let (paths, cfg) = ctx_for(temp.path());
    let mut ctx = Ctx::lazy(&paths, &cfg);
    run(
        &mut ctx,
        Request {
            host: "claude".to_string(),
            dry_run: true,
            config_dir: Some(temp.path().join("cfg")),
        },
    )
    .unwrap();
    assert!(!crate::domains::integrations::install::is_installed(
        temp.path(),
        "claude"
    ));
}
