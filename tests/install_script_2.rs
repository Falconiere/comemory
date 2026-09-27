#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
#![cfg(unix)]
//! `install.sh`, continued from `install_script.rs` (over the 300-line
//! guardrail as one file): shell rc and completion setup, run inside the
//! same daemon-safe `Rig` (#258) — a private `HOME`, data directory, and the
//! `process` supervisor, so an install here can never register a launchd
//! agent in the developer's own session.

#[path = "common/daemon_support.rs"]
mod daemon_support;
#[path = "common/install_rig.rs"]
mod install_rig;
#[path = "common/release_server.rs"]
mod release_server;

use std::path::Path;
use std::process::Output;

use install_rig::{Rig, SCRIPT};

/// `sh install.sh --dir <dir> [--no-modify-path] <args>` against `rig`'s
/// server, in its daemon-safe environment; `extra_env` is applied last so a
/// test can add `PATH`/`SHELL` overrides. `no_modify_path` is a parameter,
/// not always on, because the rc-file test needs it off.
fn run(
    rig: &Rig,
    dir: &Path,
    no_modify_path: bool,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Output {
    let mut cmd = rig.home.command_with_binary(Path::new("sh"));
    cmd.arg(SCRIPT).arg("--dir").arg(dir);
    if no_modify_path {
        cmd.arg("--no-modify-path");
    }
    cmd.args(args)
        .env("COMEMORY_RELEASES_URL", &rig.srv.base)
        .env("NO_COLOR", "1")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("ZDOTDIR")
        .env_remove("COMEMORY_INSTALL_DIR")
        .env_remove("COMEMORY_VERSION")
        .env_remove("COMEMORY_NO_MODIFY_PATH");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.output().expect("run install.sh")
}

fn ok(out: &Output) -> String {
    assert!(
        out.status.success(),
        "install.sh failed ({:?}):\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn path_line_is_appended_to_the_shell_rc_once() {
    let rig = Rig::new(&[]);
    let home = rig.home.home_dir();
    // Under `$HOME` (not the rig's private root) so install.sh's `$HOME/bin`
    // shortening in the PATH rc line applies.
    let dir = home.join("bin");
    let rc = if cfg!(target_os = "macos") {
        home.join(".bash_profile")
    } else {
        home.join(".bashrc")
    };
    let env = [("SHELL", "/bin/bash"), ("PATH", "/usr/bin:/bin")];
    let first = ok(&run(&rig, &dir, false, &[], &env));
    assert!(first.contains("added $HOME/bin to"), "{first}");
    let second = ok(&run(&rig, &dir, false, &[], &env));
    assert!(second.contains("already adds $HOME/bin"), "{second}");
    let text = std::fs::read_to_string(&rc).unwrap();
    let line = "export PATH=\"$HOME/bin:$PATH\"";
    assert_eq!(text.matches(line).count(), 1, "{text}");
    assert!(text.contains("# added by the comemory installer"));
    assert!(first.contains("open a new shell"), "{first}");
}

#[test]
fn no_modify_path_skips_the_path_line_but_keeps_completion_setup() {
    let rig = Rig::new(&[]);
    let dir = rig.dir("bin");
    let env = [("SHELL", "/bin/bash"), ("PATH", "/usr/bin:/bin")];
    let stdout = ok(&run(&rig, &dir, true, &[], &env));
    assert!(stdout.contains("PATH rc changes skipped"), "{stdout}");
    for relative in [".bashrc", ".bash_profile"] {
        let body = std::fs::read_to_string(rig.home.home_dir().join(relative)).unwrap();
        assert!(body.contains("comemory completions"), "{body}");
        assert!(!body.contains("export PATH="), "{body}");
    }
}

#[test]
fn installs_shell_completions_by_default() {
    let rig = Rig::new(&[]);
    let dir = rig.dir("bin");
    ok(&run(&rig, &dir, true, &[], &[]));
    for relative in [
        ".local/share/bash-completion/completions/comemory",
        ".local/share/zsh/site-functions/_comemory",
        ".config/fish/completions/comemory.fish",
        ".config/powershell/comemory.ps1",
    ] {
        let path = rig.home.home_dir().join(relative);
        assert!(path.is_file(), "missing completion {}", path.display());
    }
}

#[test]
fn no_completions_skips_completion_installation() {
    let rig = Rig::new(&[]);
    let dir = rig.dir("bin");
    ok(&run(&rig, &dir, true, &["--no-completions"], &[]));
    let home = rig.home.home_dir();
    assert!(!home.join(".local/share/bash-completion").exists());
    assert!(!home.join(".local/share/zsh").exists());
    assert!(!home.join(".config/fish").exists());
    assert!(!home.join(".config/powershell").exists());
}
