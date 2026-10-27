//! `domains::integrations::install` host validation and dry-run reporting against a real
//! temporary data directory.
use super::{Request, host, run, validate_host};
use crate::config::{Config, Paths};
use crate::utilities::context::Ctx;

/// Build a `Ctx` over a real temporary data directory.
fn ctx_for(dir: &std::path::Path) -> (Paths, Config) {
    (Paths::new(dir.to_path_buf()), Config::defaults())
}

#[test]
fn validate_host_accepts_the_known_hosts_and_names_the_rest() {
    for name in host::names() {
        assert_eq!(validate_host(name).unwrap(), name);
    }
    let err = validate_host("emacs").unwrap_err().to_string();
    assert!(
        err.contains("emacs"),
        "error should name the bad host: {err}"
    );
    for name in ["claude", "codex", "muse", "cursor-agent", "hermes"] {
        assert!(
            err.contains(name),
            "error should list every known host, missing {name}: {err}"
        );
    }
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
    assert_eq!(resp.plugin, Some("comemory@comemory"));
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
    // The report still names where `.mcp.json` would land, but writes none.
    let manifest = resp.mcp_manifest.unwrap();
    assert_eq!(manifest, resp.bundle.join("plugins/comemory/.mcp.json"));
    assert!(!manifest.exists());
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
fn dry_run_reports_each_hosts_own_mcp_config_and_plugin() {
    let temp = tempfile::tempdir().unwrap();
    let (paths, cfg) = ctx_for(temp.path());
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let preview = |ctx: &mut Ctx<'_>, host: &str, config: std::path::PathBuf| {
        run(
            ctx,
            Request {
                host: host.to_string(),
                dry_run: true,
                config_dir: Some(config),
            },
        )
        .unwrap()
    };

    let muse = preview(&mut ctx, "muse", temp.path().join("xdg/muse"));
    assert_eq!(
        muse.mcp_manifest,
        Some(muse.bundle.join("plugins/comemory/.mcp.json")),
        "muse's manifest launches bin/mcp-server.sh, which reads this file"
    );
    assert!(muse.next_steps.iter().any(|step| step.contains("approve")));

    let cursor = preview(&mut ctx, "cursor-agent", temp.path().join("cursor"));
    assert_eq!(
        cursor.mcp_manifest,
        Some(temp.path().join("cursor/mcp.json"))
    );
    assert_eq!(cursor.plugin, None, "cursor has no plugin manager record");

    let hermes = preview(&mut ctx, "hermes", temp.path().join("hermes"));
    assert_eq!(hermes.mcp_manifest, None, "hermes has no MCP config");
    assert_eq!(hermes.delivers, "skills");
    assert!(!temp.path().join("cursor").exists());
    assert!(!temp.path().join("hermes").exists());
}

#[test]
fn the_installed_marker_is_per_host_not_per_shared_bundle() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path();

    // Nothing installed yet.
    for name in host::names() {
        assert!(!crate::domains::integrations::install::is_installed(
            data, name
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
