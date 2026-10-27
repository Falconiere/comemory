//! The host table: lookup, config-dir scoping, the muse override rule, and
//! the per-host report facts `install` and `setup` render.
use std::path::Path;

use super::{Mechanism, Presence, TABLE, absence_reason, lookup, names};

#[test]
fn lookup_finds_every_host_and_names_the_rest() {
    for host in TABLE {
        assert_eq!(lookup(host.name).unwrap().name, host.name);
    }
    let err = lookup("emacs").unwrap_err().to_string();
    assert!(
        err.contains("emacs"),
        "error should name the bad host: {err}"
    );
    for name in names() {
        assert!(err.contains(name), "error should list {name}: {err}");
    }
}

#[test]
fn scope_env_scopes_cli_hosts_and_leaves_file_hosts_alone() {
    assert_eq!(
        lookup("muse")
            .unwrap()
            .scope_env(Path::new("/tmp/xdg/muse")),
        Some(("XDG_DATA_HOME", Path::new("/tmp/xdg").to_path_buf()))
    );
    assert_eq!(
        lookup("claude")
            .unwrap()
            .scope_env(Path::new("/tmp/claude")),
        Some(("CLAUDE_CONFIG_DIR", Path::new("/tmp/claude").to_path_buf()))
    );
    assert_eq!(
        lookup("codex").unwrap().scope_env(Path::new("/tmp/codex")),
        Some(("CODEX_HOME", Path::new("/tmp/codex").to_path_buf()))
    );
    for file_host in ["cursor-agent", "hermes"] {
        assert_eq!(
            lookup(file_host).unwrap().scope_env(Path::new("/tmp/x")),
            None
        );
    }
}

#[test]
fn an_explicit_config_dir_wins_and_is_absolutized() {
    let temp = tempfile::tempdir().unwrap();
    let explicit = temp.path().join("explicit");
    let resolved = lookup("claude")
        .unwrap()
        .config_dir(Some(explicit.clone()))
        .unwrap();
    assert!(resolved.is_absolute());
    assert_eq!(resolved, explicit);
}

#[test]
fn a_muse_config_dir_is_the_muse_directory_itself() {
    let temp = tempfile::tempdir().unwrap();
    let muse = lookup("muse").unwrap();
    let dir = temp.path().join("share").join("muse");
    assert_eq!(muse.config_dir(Some(dir.clone())).unwrap(), dir);
    assert_eq!(
        muse.scope_env(&dir),
        Some(("XDG_DATA_HOME", temp.path().join("share")))
    );

    // Muse is scoped through its XDG base, so any other leaf would install
    // where muse never reads: refused, naming the expected leaf.
    let err = muse
        .config_dir(Some(temp.path().join("share")))
        .unwrap_err()
        .to_string();
    assert!(err.contains("`muse` directory"), "unexpected error: {err}");
}

#[test]
fn only_plugin_manager_hosts_get_hooks_and_a_plugin_id() {
    for host in TABLE {
        let manager = matches!(
            host.mechanism,
            Mechanism::PluginCli { .. } | Mechanism::MusePlugins
        );
        assert_eq!(host.installs_hooks(), manager, "{}", host.name);
        assert_eq!(
            host.plugin(),
            manager.then_some("comemory@comemory"),
            "{}",
            host.name
        );
        assert!(!host.next_steps().is_empty(), "{}", host.name);
    }
    assert_eq!(lookup("hermes").unwrap().delivers(), "skills");
    assert_eq!(
        lookup("cursor-agent").unwrap().delivers(),
        "skills and MCP server"
    );
}

#[test]
fn muse_next_steps_name_the_approve_command() {
    let steps = lookup("muse").unwrap().next_steps().join("\n");
    assert!(
        steps.contains("muse plugins approve comemory"),
        "muse holds hooks for review; the report must say how to approve: {steps}"
    );
    let claude = lookup("claude").unwrap().next_steps().join("\n");
    assert!(
        claude.contains("comemory@toolu"),
        "plugin hosts keep the toolu note: {claude}"
    );
    let hermes = lookup("hermes").unwrap().next_steps().join("\n");
    assert!(
        !hermes.contains("toolu") && !hermes.contains("plugin"),
        "a file host gets no plugin advice: {hermes}"
    );
}

#[test]
fn absence_reason_follows_each_hosts_presence_probe() {
    assert!(absence_reason(Some("hermes")).contains("no hermes config dir"));
    assert_eq!(
        absence_reason(Some("claude")),
        "no claude CLI found on PATH"
    );
    let all = absence_reason(None);
    for host in TABLE {
        assert!(
            all.contains(host.name),
            "roster must name {}: {all}",
            host.name
        );
    }
    let cli_hosts = TABLE
        .iter()
        .filter(|host| host.presence == Presence::Cli)
        .count();
    assert_eq!(cli_hosts, 4, "only hermes is probed by config dir");
}
