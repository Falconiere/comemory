//! `api::install` host validation and dry-run reporting against a real
//! temporary data directory.
use super::{HOSTS, Request, config_dir, run, validate_host};
use crate::api::Ctx;
use crate::config::{Config, Paths};

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
