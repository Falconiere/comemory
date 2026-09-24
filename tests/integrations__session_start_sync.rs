#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The agent plugin's real `SessionStart` hook is a sync trigger: run by
//! `bash` with a real payload whose `cwd` is nowhere near the repo, it still
//! gets a stale hooked repo re-indexed — through the built binary, found on
//! a sandboxed `PATH` — and still prints its own JSON unchanged.

#[path = "common/git_repo.rs"]
mod git_repo;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::cargo::CommandCargoExt as _;
use comemory::utilities::file_lock::FileLock;
use tempfile::TempDir;

/// Environment a host session may have set that would point the hook at the
/// real config directory or a different host.
const HOST_VARS: &[&str] = &[
    "TOOLU_CONFIG_DIR",
    "TOOLU_HOST_OVERRIDE",
    "PLUGIN_ROOT",
    "CLAUDE_PLUGIN_ROOT",
    "CODEX_HOME",
];

struct Sandbox {
    home: TempDir,
}

impl Sandbox {
    /// A temp `HOME` whose `bin/` holds the built binary and `jq` (the hook
    /// needs it), and an empty `elsewhere/` to run from.
    fn new() -> Self {
        let home = TempDir::new().unwrap();
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let built = Command::cargo_bin("comemory").unwrap();
        std::os::unix::fs::symlink(built.get_program(), bin.join("comemory")).unwrap();
        let jq = which("jq").expect("the SessionStart hook needs jq on the test host");
        std::os::unix::fs::symlink(jq, bin.join("jq")).unwrap();
        std::fs::create_dir_all(home.path().join("elsewhere")).unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        Self { home }
    }

    fn path_var(&self) -> String {
        format!("{}:/usr/bin:/bin", self.home.path().join("bin").display())
    }

    fn env(&self, cmd: &mut Command) {
        for var in HOST_VARS {
            cmd.env_remove(var);
        }
        cmd.env("HOME", self.home.path())
            .env("PATH", self.path_var())
            .env("CLAUDE_CONFIG_DIR", self.home.path().join(".claude"))
            .env("COMEMORY_DATA_DIR", self.home.path().join("data"))
            .current_dir(self.home.path().join("elsewhere"));
    }

    fn comemory(&self, args: &[&str]) -> serde_json::Value {
        let mut cmd = Command::cargo_bin("comemory").unwrap();
        self.env(&mut cmd);
        let out = cmd.args(args).output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        serde_json::from_slice(&out.stdout).unwrap()
    }

    fn git(&self, repo: &Path, args: &[&str]) {
        let mut cmd = Command::new("git");
        self.env(&mut cmd);
        let out = cmd.args(args).current_dir(repo).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", stderr(&out));
    }

    fn last_head(&self, label: &str) -> Option<String> {
        self.comemory(&["repos", "--json"])["repos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["repo"] == label)
            .and_then(|r| r["last_head"].as_str().map(str::to_string))
    }

    fn wait_for_head(&self, label: &str, want: Option<&str>) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let head = self.last_head(label);
            if head.is_some_and(|h| want.is_none_or(|w| w == h)) {
                return;
            }
            assert!(Instant::now() < deadline, "{label} never reached {want:?}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// A hooked, registered repo with one more commit no hook saw.
    fn stale_repo(&self, name: &str) -> (PathBuf, String) {
        let root = self.home.path().join(name);
        git_repo::init_repo(&root);
        std::fs::write(root.join("lib.rs"), "pub fn one() {}\n").unwrap();
        self.git(&root, &["add", "-A"]);
        self.git(&root, &["commit", "-q", "-m", "init"]);
        self.comemory(&["install-hooks", "--repo", root.to_str().unwrap(), "--json"]);
        self.wait_for_head(name, None);
        drop(FileLock::acquire(&self.home.path().join("data").join("sync.lock"), "test").unwrap());
        std::fs::write(root.join("two.rs"), "pub fn two() {}\n").unwrap();
        self.git(&root, &["add", "-A"]);
        self.git(
            &root,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-q",
                "-m",
                "hookless",
            ],
        );
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&root)
            .output()
            .unwrap();
        let head = String::from_utf8(out.stdout).unwrap().trim().to_string();
        (root, head)
    }

    /// Run the real hook the way the host does: payload on stdin.
    fn session_start(&self) -> Output {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("integrations/agent/hooks/session-start.sh");
        let payload = serde_json::json!({
            "cwd": self.home.path().join("elsewhere"),
            "session_id": "session-start-sync-test",
            "hook_event_name": "SessionStart",
        });
        let mut cmd = Command::new("bash");
        self.env(&mut cmd);
        let mut child = cmd
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

fn which(tool: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(tool))
            .find(|p| p.is_file())
    })
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn session_start_refreshes_a_stale_hooked_repo_and_keeps_its_own_output() {
    let sb = Sandbox::new();
    let (_repo, head) = sb.stale_repo("far-away");
    assert_ne!(sb.last_head("far-away").as_deref(), Some(head.as_str()));

    let out = sb.session_start();

    assert!(out.status.success(), "{}", stderr(&out));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["hookSpecificOutput"]["hookEventName"], "SessionStart");
    assert!(
        json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .starts_with("Comemory:"),
        "{json}"
    );
    sb.wait_for_head("far-away", Some(&head));
}

#[test]
fn a_session_with_comemory_disabled_triggers_nothing() {
    let sb = Sandbox::new();
    let (_repo, head) = sb.stale_repo("left-alone");
    std::fs::write(
        sb.home.path().join(".claude").join("comemory.json"),
        r#"{"skills":{"comemory":false}}"#,
    )
    .unwrap();

    let out = sb.session_start();

    assert!(out.status.success(), "{}", stderr(&out));
    std::thread::sleep(Duration::from_secs(3));
    assert_ne!(
        sb.last_head("left-alone").as_deref(),
        Some(head.as_str()),
        "a disabled plugin must not launch a pass"
    );
}
