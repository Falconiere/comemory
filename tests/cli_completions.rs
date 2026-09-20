#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory completions <shell>`.

use assert_cmd::Command;
use tempfile::TempDir;

const BLOCK_START: &str = "# >>> comemory completions >>>";
const BLOCK_END: &str = "# <<< comemory completions <<<";

fn run_completions(shell: &str) -> assert_cmd::assert::Assert {
    Command::cargo_bin("comemory")
        .expect("cargo_bin comemory")
        .args(["completions", shell])
        .assert()
}

fn run_install(home: &TempDir) -> assert_cmd::assert::Assert {
    Command::cargo_bin("comemory")
        .expect("cargo_bin comemory")
        .args(["completions", "--install"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("ZDOTDIR", home.path().join("zsh"))
        .assert()
}

#[test]
fn fish_emits_completion_script() {
    let out = run_completions("fish")
        .success()
        .get_output()
        .stdout
        .clone();
    let body = String::from_utf8(out).expect("fish completions are utf-8");
    assert!(!body.trim().is_empty(), "fish completions stdout is empty");
    assert!(
        body.contains("comemory"),
        "fish completions missing binary name"
    );
}

#[test]
fn bash_emits_completion_script() {
    let out = run_completions("bash")
        .success()
        .get_output()
        .stdout
        .clone();
    let body = String::from_utf8(out).expect("bash completions are utf-8");
    assert!(!body.trim().is_empty(), "bash completions stdout is empty");
    assert!(
        body.contains("comemory"),
        "bash completions missing binary name"
    );
}

#[test]
fn zsh_emits_completion_script() {
    let out = run_completions("zsh").success().get_output().stdout.clone();
    let body = String::from_utf8(out).expect("zsh completions are utf-8");
    assert!(!body.trim().is_empty(), "zsh completions stdout is empty");
    assert!(
        body.contains("comemory"),
        "zsh completions missing binary name"
    );
}

#[test]
fn powershell_emits_completion_script() {
    let out = run_completions("powershell")
        .success()
        .get_output()
        .stdout
        .clone();
    let body = String::from_utf8(out).expect("powershell completions are utf-8");
    assert!(
        !body.trim().is_empty(),
        "powershell completions stdout is empty"
    );
    assert!(
        body.contains("comemory"),
        "powershell completions missing binary name"
    );
}

#[test]
fn elvish_emits_completion_script() {
    let out = run_completions("elvish")
        .success()
        .get_output()
        .stdout
        .clone();
    let body = String::from_utf8(out).expect("elvish completions are utf-8");
    assert!(
        !body.trim().is_empty(),
        "elvish completions stdout is empty"
    );
    assert!(
        body.contains("comemory"),
        "elvish completions missing binary name"
    );
}

#[test]
fn install_writes_and_registers_the_four_supported_shells() {
    let home = TempDir::new().expect("temp home");

    run_install(&home).success();

    for (relative, marker) in [
        (
            "data/bash-completion/completions/comemory",
            "complete -F _comemory",
        ),
        ("data/zsh/site-functions/_comemory", "#compdef comemory"),
        (
            "config/fish/completions/comemory.fish",
            "complete -c comemory",
        ),
        (
            "config/powershell/comemory.ps1",
            "Register-ArgumentCompleter",
        ),
    ] {
        let path = home.path().join(relative);
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read installed completion {}: {e}", path.display()));
        assert!(!body.trim().is_empty(), "{} is empty", path.display());
        assert!(
            body.contains(marker),
            "{} is missing {marker}",
            path.display()
        );
    }

    for relative in [
        ".bashrc",
        ".bash_profile",
        "zsh/.zshrc",
        "config/powershell/profile.ps1",
    ] {
        let path = home.path().join(relative);
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read configured profile {}: {e}", path.display()));
        assert_eq!(body.matches(BLOCK_START).count(), 1, "{}", path.display());
        assert_eq!(body.matches(BLOCK_END).count(), 1, "{}", path.display());
    }
}

#[test]
fn install_json_reports_scripts_and_profiles() {
    let home = TempDir::new().expect("temp home");
    let out = Command::cargo_bin("comemory")
        .expect("cargo_bin comemory")
        .args(["--json", "completions", "--install"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("ZDOTDIR", home.path().join("zsh"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: serde_json::Value = serde_json::from_slice(&out).expect("JSON report");

    assert_eq!(report["installed"].as_array().map(Vec::len), Some(4));
    assert_eq!(report["profiles"].as_array().map(Vec::len), Some(4));
}

#[test]
fn install_is_idempotent_and_preserves_profile_text() {
    let home = TempDir::new().expect("temp home");
    let zsh_dir = home.path().join("zsh");
    std::fs::create_dir_all(&zsh_dir).expect("create zsh config dir");
    let zshrc = zsh_dir.join(".zshrc");
    std::fs::write(&zshrc, "# user's setup\nexport EDITOR=vi\n").expect("seed zshrc");

    run_install(&home).success();
    run_install(&home).success();

    let body = std::fs::read_to_string(&zshrc).expect("read zshrc");
    assert!(body.starts_with(&format!(
        "# user's setup\nexport EDITOR=vi\n\n{BLOCK_START}\n"
    )));
    assert_eq!(body.matches(BLOCK_START).count(), 1);
    assert_eq!(body.matches(BLOCK_END).count(), 1);
}

#[test]
fn install_collapses_duplicate_managed_blocks() {
    let home = TempDir::new().expect("temp home");
    let zsh_dir = home.path().join("zsh");
    std::fs::create_dir_all(&zsh_dir).expect("create zsh config dir");
    let zshrc = zsh_dir.join(".zshrc");
    std::fs::write(
        &zshrc,
        format!(
            "export EDITOR=vi\n\n{BLOCK_START}\nold first\n{BLOCK_END}\n\nexport PATH\n\n{BLOCK_START}\nold second\n{BLOCK_END}\n"
        ),
    )
    .expect("seed duplicate completion blocks");

    run_install(&home).success();

    let body = std::fs::read_to_string(&zshrc).expect("read zshrc");
    assert_eq!(body.matches(BLOCK_START).count(), 1);
    assert_eq!(body.matches(BLOCK_END).count(), 1);
    assert!(body.contains("export EDITOR=vi\n"));
    assert!(body.contains("export PATH\n"));
    assert!(!body.contains("old first"));
    assert!(!body.contains("old second"));
}

#[test]
fn install_uses_an_existing_bash_login_profile() {
    let home = TempDir::new().expect("temp home");
    let profile = home.path().join(".profile");
    std::fs::write(&profile, "export EDITOR=vi\n").expect("seed profile");

    run_install(&home).success();

    assert!(
        !home.path().join(".bash_profile").exists(),
        "creating .bash_profile would stop Bash from loading .profile"
    );
    let body = std::fs::read_to_string(&profile).expect("read profile");
    assert!(body.starts_with("export EDITOR=vi\n"));
    assert!(body.contains("# >>> comemory completions >>>"));
}

#[cfg(unix)]
#[test]
fn install_atomically_replaces_a_read_only_completion_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = TempDir::new().expect("temp home");
    let bash = home
        .path()
        .join("data/bash-completion/completions/comemory");
    std::fs::create_dir_all(bash.parent().expect("bash completion parent"))
        .expect("create bash completion dir");
    std::fs::write(&bash, "stale completion\n").expect("seed completion");
    std::fs::set_permissions(&bash, std::fs::Permissions::from_mode(0o444))
        .expect("make completion read-only");

    run_install(&home).success();

    let body = std::fs::read_to_string(&bash).expect("read refreshed completion");
    assert!(body.contains("complete -F _comemory"), "{body}");
}

#[cfg(unix)]
#[test]
fn install_preserves_existing_profile_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = TempDir::new().expect("temp home");
    let zsh_dir = home.path().join("zsh");
    std::fs::create_dir_all(&zsh_dir).expect("create zsh config dir");
    let zshrc = zsh_dir.join(".zshrc");
    std::fs::write(&zshrc, "export PRIVATE_TOKEN=secret\n").expect("seed zshrc");
    std::fs::set_permissions(&zshrc, std::fs::Permissions::from_mode(0o600))
        .expect("make zshrc private");

    run_install(&home).success();

    let mode = std::fs::metadata(&zshrc)
        .expect("zshrc metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[cfg(unix)]
#[test]
fn install_updates_a_symlinked_profile_without_replacing_the_link() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new().expect("temp home");
    let zsh_dir = home.path().join("zsh");
    std::fs::create_dir_all(&zsh_dir).expect("create zsh config dir");
    let target = home.path().join("dotfiles-zshrc");
    std::fs::write(&target, "export EDITOR=vi\n").expect("seed zshrc target");
    let zshrc = zsh_dir.join(".zshrc");
    symlink(&target, &zshrc).expect("symlink zshrc");

    run_install(&home).success();

    assert!(
        std::fs::symlink_metadata(&zshrc)
            .expect("zshrc symlink metadata")
            .file_type()
            .is_symlink()
    );
    let body = std::fs::read_to_string(&target).expect("read zshrc target");
    assert!(body.contains("# >>> comemory completions >>>"));
}

#[test]
fn install_rejects_a_shell_positional() {
    Command::cargo_bin("comemory")
        .expect("cargo_bin comemory")
        .args(["completions", "--install", "bash"])
        .assert()
        .failure();
}
