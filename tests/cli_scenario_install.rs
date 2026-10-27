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

/// PreCompact under the muse override keeps the plain `systemMessage`
/// shape, never the Claude `hookSpecificOutput` envelope.
#[cfg(unix)]
#[test]
fn precompact_uses_plain_shape_for_muse() {
    use std::io::Write as _;

    let payload = r#"{"hook_event_name":"PreCompact","session_id":"abc123","cwd":"/tmp/x"}"#;
    let mut child = std::process::Command::new("bash")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/integrations/agent/hooks/memory-lifecycle.sh"
        ))
        .env("TOOLU_HOST_OVERRIDE", "muse")
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
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("systemMessage"),
        "muse PreCompact must use the plain shape: {stdout}"
    );
    assert!(
        !stdout.contains("hookSpecificOutput"),
        "muse PreCompact must not use the Claude envelope: {stdout}"
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
/// binary, and the `cursor` alias resolves. Mirrors the claude preview test.
#[test]
fn installation_preview_covers_new_hosts_and_cursor_alias() {
    for host in ["muse", "cursor-agent", "hermes", "cursor"] {
        let home = tempfile::tempdir().unwrap();
        Command::new(assert_cmd::cargo::cargo_bin!("comemory"))
            .args(["install", host, "--dry-run", "--json", "--data-dir"])
            .arg(home.path().join("data"))
            .arg("--config-dir")
            .arg(home.path().join("host"))
            .assert()
            .success()
            .stdout(contains("comemory@comemory"));
        assert!(
            !home.path().join("data").exists(),
            "host {host} must not write a data dir on preview"
        );
        assert!(
            !home.path().join("host").exists(),
            "host {host} must not write a config dir on preview"
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
