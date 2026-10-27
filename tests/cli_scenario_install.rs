#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Real-binary installation preview and offline asset distribution checks.
use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn installation_preview_does_not_write_or_require_a_host() {
    let home = tempfile::tempdir().unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("comemory"))
        .args(["install", "claude", "--dry-run", "--json", "--data-dir"])
        .arg(home.path().join("data"))
        .arg("--config-dir")
        .arg(home.path().join("host"))
        .assert()
        .success()
        .stdout(contains("comemory@comemory"));
    assert!(!home.path().join("data").exists());
    assert!(!home.path().join("host").exists());
}

/// AC-7: a dry-run install reports where `.mcp.json` would land under the
/// plugin root, without requiring the real `claude` CLI or writing anything.
/// The real write (and the exact `command`/`args` it carries for the test
/// binary) is exercised end to end by `scripts/test-agent-install.sh`, the
/// only journey that has the host CLI to install against.
#[test]
fn install_writes_mcp_manifest() {
    let home = tempfile::tempdir().unwrap();
    let assert = Command::new(assert_cmd::cargo::cargo_bin!("comemory"))
        .args(["install", "claude", "--dry-run", "--json", "--data-dir"])
        .arg(home.path().join("data"))
        .arg("--config-dir")
        .arg(home.path().join("host"))
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let manifest = report["mcp_manifest"].as_str().unwrap();
    let expected = home
        .path()
        .join("data")
        .join("integrations")
        .join(env!("CARGO_PKG_VERSION"))
        .join("plugins")
        .join("comemory")
        .join(".mcp.json");
    assert_eq!(
        std::path::Path::new(manifest),
        expected.as_path(),
        "mcp_manifest must be the plugin-root path under this data dir"
    );
    assert!(!std::path::Path::new(manifest).exists());
}

/// File-based hosts install for real without invoking any host CLI: skills
/// arrive as links into the versioned bundle, cursor gets its `mcp.json`
/// entry, a rerun is clean, and the report makes no plugin or hook claims.
/// `PATH` is inherited because the skill wrappers' own tools (bash, git, jq)
/// are probed.
#[cfg(unix)]
#[test]
fn file_hosts_install_for_real_without_a_host_cli() {
    for (host, has_mcp) in [("hermes", false), ("cursor-agent", true)] {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(host);
        for _ in 0..2 {
            let output = Command::new(assert_cmd::cargo::cargo_bin!("comemory"))
                .args(["install", host, "--data-dir"])
                .arg(home.path().join("data"))
                .arg("--config-dir")
                .arg(&config)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(output.status.success(), "{host}: {output:?}");
            assert!(stdout.contains("Installed comemory skills"), "{stdout}");
            assert!(
                !stdout.contains("Plugin:")
                    && !stdout.contains("toolu")
                    && !stdout.contains("hooks"),
                "{host} report must not claim a plugin or hooks: {stdout}"
            );
        }
        let link = config.join("skills/agent-memory");
        assert!(
            link.symlink_metadata().unwrap().file_type().is_symlink(),
            "{host} skills are links into the bundle"
        );
        assert!(
            std::fs::read_link(&link)
                .unwrap()
                .starts_with(home.path().join("data/integrations"))
        );
        assert!(link.join("scripts/comemory.sh").is_file());
        assert_eq!(config.join("mcp.json").is_file(), has_mcp, "{host}");
        assert!(
            home.path()
                .join("data/integrations")
                .join(env!("CARGO_PKG_VERSION"))
                .join(format!(".installed-{host}"))
                .is_file()
        );
    }
}

/// Muse's MCP launcher runs the absolute binary the install recorded in the
/// bundle's `.mcp.json` — the binary that installed the hooks — and only
/// falls back to `comemory` on `PATH` when that manifest names nothing
/// runnable.
#[cfg(unix)]
#[test]
fn muse_mcp_launcher_runs_the_installing_binary() {
    use std::os::unix::fs::PermissionsExt as _;

    let task = tempfile::tempdir().unwrap();
    let plugin = task.path().join("plugin");
    std::fs::create_dir_all(plugin.join("bin")).unwrap();
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/integrations/agent/bin/mcp-server.sh"
        ),
        plugin.join("bin/mcp-server.sh"),
    )
    .unwrap();
    let stub = |dir: &std::path::Path, label: &str| {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("comemory");
        std::fs::write(&path, format!("#!/bin/sh\necho {label} \"$@\"\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let installed = stub(&task.path().join("installed"), "installed");
    let on_path = stub(&task.path().join("on-path"), "on-path");
    let launch = || {
        let output = std::process::Command::new("bash")
            .arg(plugin.join("bin/mcp-server.sh"))
            .env(
                "PATH",
                format!(
                    "{}:/usr/bin:/bin:/opt/homebrew/bin",
                    on_path.parent().unwrap().display()
                ),
            )
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    };

    std::fs::write(
        plugin.join(".mcp.json"),
        serde_json::json!({"mcpServers": {"comemory": {"command": installed, "args": ["mcp"]}}})
            .to_string(),
    )
    .unwrap();
    assert_eq!(launch().trim(), "installed mcp");

    std::fs::remove_file(plugin.join(".mcp.json")).unwrap();
    assert_eq!(launch().trim(), "on-path mcp");
}

#[cfg(unix)]
#[test]
fn project_skill_lifecycle_uses_real_repository_paths() {
    Command::new("bash")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/test-project-skills.sh"
        ))
        .assert()
        .success();
}

/// AC-8: the `SessionEnd` capture gate. With a host override that has no
/// supported transcript path (`muse`, and the long-standing `codex`
/// precedent), the hook exits silently without invoking `comemory` — proved
/// by a fake `comemory` on `PATH` that would leave a sentinel file.
/// Without an override (the Claude path) the same fake records an invocation.
#[cfg(unix)]
#[test]
fn session_end_capture_is_gated_by_host_override() {
    use std::io::Write as _;

    let task = tempfile::tempdir().unwrap();
    let bin = task.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let sentinel = task.path().join("comemory-invoked");
    let fake = bin.join("comemory");
    let mut script = std::fs::File::create(&fake).unwrap();
    writeln!(
        script,
        "#!/bin/sh\ntouch \"{}\"",
        sentinel.to_str().unwrap()
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let path_value = std::env::join_paths(paths).unwrap();
    let hook = std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/integrations/agent/hooks/session-end.sh"
    ));
    let payload = r#"{"hook_event_name":"SessionEnd","session_id":"abc123","cwd":"/tmp/x","transcript_path":"/tmp/x.jsonl"}"#;

    for host in ["muse", "codex"] {
        let _ = std::fs::remove_file(&sentinel);
        let mut child = std::process::Command::new("bash")
            .arg(&hook)
            .env("PATH", &path_value)
            .env("HOME", task.path())
            .env("CLAUDE_CONFIG_DIR", task.path().join("claude"))
            .env_remove("TOOLU_CONFIG_DIR")
            .env("TOOLU_HOST_OVERRIDE", host)
            .env_remove("PLUGIN_ROOT")
            .env_remove("CLAUDE_PLUGIN_ROOT")
            .env_remove("MUSE_PLUGIN_ROOT")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert!(
            !sentinel.exists(),
            "override {host} must not invoke comemory"
        );
    }

    let _ = std::fs::remove_file(&sentinel);
    let mut child = std::process::Command::new("bash")
        .arg(&hook)
        .env("PATH", &path_value)
        .env("HOME", task.path())
        .env("CLAUDE_CONFIG_DIR", task.path().join("claude"))
        .env_remove("TOOLU_CONFIG_DIR")
        .env_remove("TOOLU_HOST_OVERRIDE")
        .env_remove("PLUGIN_ROOT")
        .env_remove("CLAUDE_PLUGIN_ROOT")
        .env_remove("MUSE_PLUGIN_ROOT")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !sentinel.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        sentinel.exists(),
        "the Claude path must still invoke comemory capture"
    );
}

/// Run one bundled hook script with `payload` on stdin under the muse
/// override, isolated from the developer's own config, returning stdout.
#[cfg(unix)]
fn run_muse_hook(script: &str, payload: &str, home: &std::path::Path) -> String {
    use std::io::Write as _;

    let mut child = std::process::Command::new("bash")
        .arg(format!(
            "{}/integrations/agent/hooks/{script}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .env("TOOLU_HOST_OVERRIDE", "muse")
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("PATH", "/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin")
        .env_remove("TOOLU_CONFIG_DIR")
        .env_remove("PLUGIN_ROOT")
        .env_remove("CLAUDE_PLUGIN_ROOT")
        .env_remove("MUSE_PLUGIN_ROOT")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{script} failed");
    String::from_utf8(output.stdout).unwrap()
}

/// PreCompact under the muse override keeps the plain `systemMessage`
/// shape, never the Claude `hookSpecificOutput` envelope — driven through
/// `pre-compact.sh`, the entry point the muse manifest registers (muse
/// refuses one hook source under two events).
#[cfg(unix)]
#[test]
fn precompact_uses_plain_shape_for_muse() {
    let home = tempfile::tempdir().unwrap();
    let payload = r#"{"hook_event_name":"PreCompact","session_id":"abc123","cwd":"/tmp/x"}"#;
    for script in ["memory-lifecycle.sh", "pre-compact.sh"] {
        let stdout = run_muse_hook(script, payload, home.path());
        assert!(
            stdout.contains("systemMessage"),
            "muse PreCompact via {script} must use the plain shape: {stdout}"
        );
        assert!(
            !stdout.contains("hookSpecificOutput"),
            "muse PreCompact via {script} must not use the Claude envelope: {stdout}"
        );
    }
}

/// The muse Stop entry point reaches memory-lifecycle.sh's Stop branch:
/// with no `comemory` on `PATH` it exits cleanly and silently, never
/// treating the event as unknown or failing the host's Stop.
#[cfg(unix)]
#[test]
fn stop_entry_point_dispatches_to_the_lifecycle_hook() {
    let home = tempfile::tempdir().unwrap();
    let payload = r#"{"hook_event_name":"Stop","session_id":"abc123","cwd":"/tmp/x"}"#;
    let stdout = run_muse_hook("stop.sh", payload, home.path());
    assert!(
        !stdout.contains("hookSpecificOutput"),
        "Stop never emits the Claude envelope: {stdout}"
    );
}

/// The foundation path routing follows the same override: `muse` resolves
/// under the muse config home, `codex` under `CODEX_HOME`, and the default
/// under `CLAUDE_CONFIG_DIR`, so hooks never read or write another host's
/// toggle file.
#[cfg(unix)]
#[test]
fn foundation_routes_config_home_per_host_override() {
    let task = tempfile::tempdir().unwrap();
    let lib = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/integrations/agent/lib/project-skills-foundation.sh"
    );
    let check = |override_value: Option<&str>, extra_env: &[(&str, &str)]| {
        let mut command = std::process::Command::new("bash");
        command.arg("-c").arg(format!(
            ". {lib}; ps_config_root; printf '|'; ps_project_dirname"
        ));
        // A developer's own overrides would otherwise win over the routing
        // under test.
        for key in [
            "TOOLU_CONFIG_DIR",
            "TOOLU_PROJECT_CONFIG_DIRNAME",
            "PLUGIN_ROOT",
            "CODEX_HOME",
            "CLAUDE_CONFIG_DIR",
            "XDG_CONFIG_HOME",
        ] {
            command.env_remove(key);
        }
        match override_value {
            Some(host) => {
                command.env("TOOLU_HOST_OVERRIDE", host);
            }
            None => {
                command.env_remove("TOOLU_HOST_OVERRIDE");
            }
        }
        for (key, value) in extra_env {
            command.env(key, value);
        }
        let output = command.output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    let xdg = task.path().join("xdg");
    let got = check(
        Some("muse"),
        &[
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
            ("HOME", task.path().to_str().unwrap()),
        ],
    );
    assert_eq!(got, format!("{}/muse|.muse", xdg.to_str().unwrap()));
    let got = check(
        Some("codex"),
        &[
            (
                "CODEX_HOME",
                task.path().join("codex-home").to_str().unwrap(),
            ),
            ("HOME", task.path().to_str().unwrap()),
        ],
    );
    assert!(got.starts_with(task.path().join("codex-home").to_str().unwrap()));
    assert!(
        std::path::Path::new(&got)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("codex"))
    );
}

/// AC-1/AC-7: every new host previews without writing or requiring its
/// binary, and the `cursor` alias resolves. Muse's config dir is its own
/// `muse` directory; only plugin-manager hosts report a plugin id, and only
/// MCP-capable hosts an MCP config.
#[test]
fn installation_preview_covers_new_hosts_and_cursor_alias() {
    for (host, leaf, plugin, has_mcp) in [
        ("muse", "muse", true, true),
        ("cursor-agent", "host", false, true),
        ("hermes", "host", false, false),
        ("cursor", "host", false, true),
    ] {
        let home = tempfile::tempdir().unwrap();
        let output = Command::new(assert_cmd::cargo::cargo_bin!("comemory"))
            .args(["install", host, "--dry-run", "--json", "--data-dir"])
            .arg(home.path().join("data"))
            .arg("--config-dir")
            .arg(home.path().join("cfg").join(leaf))
            .output()
            .unwrap();
        assert!(output.status.success(), "{host}: {output:?}");
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report["plugin"].as_str() == Some("comemory@comemory"),
            plugin,
            "{host}: {report}"
        );
        assert_eq!(
            !report["mcp_manifest"].is_null(),
            has_mcp,
            "{host}: {report}"
        );
        assert!(
            report["next_steps"]
                .as_array()
                .is_some_and(|steps| !steps.is_empty()),
            "{host}: {report}"
        );
        assert!(
            !home.path().join("data").exists(),
            "host {host} must not write a data dir on preview"
        );
        assert!(
            !home.path().join("cfg").exists(),
            "host {host} must not write a config dir on preview"
        );
    }
}

/// Muse is scoped through `XDG_DATA_HOME`, so a `--config-dir` that is not
/// a `muse` directory is refused, and an empty or relative `XDG_DATA_HOME`
/// counts as unset (XDG spec) instead of resolving against the cwd.
#[test]
fn muse_config_dir_follows_the_xdg_rules() {
    let home = tempfile::tempdir().unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("comemory"))
        .args(["install", "muse", "--dry-run", "--json", "--data-dir"])
        .arg(home.path().join("data"))
        .arg("--config-dir")
        .arg(home.path().join("share"))
        .assert()
        .failure()
        .code(64)
        .stderr(contains("`muse` directory"));

    for xdg in ["", "relative/share"] {
        let output = Command::new(assert_cmd::cargo::cargo_bin!("comemory"))
            .args(["install", "muse", "--dry-run", "--json", "--data-dir"])
            .arg(home.path().join("data"))
            .env("HOME", home.path())
            .env("XDG_DATA_HOME", xdg)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report["config_dir"].as_str().unwrap(),
            home.path().join(".local/share/muse").to_str().unwrap(),
            "XDG_DATA_HOME={xdg:?} must fall back to ~/.local/share"
        );
    }
}

/// AC-7: an unknown host is refused by clap before touching the filesystem,
/// naming the bad value and every known host.
#[test]
fn unknown_host_install_is_refused_with_the_full_roster() {
    let home = tempfile::tempdir().unwrap();
    let assert = Command::new(assert_cmd::cargo::cargo_bin!("comemory"))
        .args(["install", "emacs", "--dry-run", "--json", "--data-dir"])
        .arg(home.path().join("data"))
        .arg("--config-dir")
        .arg(home.path().join("host"))
        .assert()
        .failure()
        .code(2);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap()
        + &String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    for host in ["claude", "codex", "muse", "cursor-agent", "hermes", "emacs"] {
        assert!(output.contains(host), "refusal must name {host}: {output}");
    }
    assert!(!home.path().join("data").exists());
    assert!(!home.path().join("host").exists());
}

#[cfg(unix)]
#[test]
fn badge_hook_without_home_or_host_configuration_is_nonfatal() {
    let binary = assert_cmd::cargo::cargo_bin!("comemory");
    let mut paths = vec![binary.parent().unwrap().to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    Command::new("bash")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/integrations/agent/hooks/comemory-status.sh"
        ))
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env_remove("HOME")
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("TOOLU_CONFIG_DIR")
        .env_remove("TOOLU_HOST_OVERRIDE")
        .env_remove("PLUGIN_ROOT")
        .write_stdin("{}")
        .assert()
        .success()
        .stdout("")
        .stderr("");
}
