#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of `GET|POST /api/v1/hooks` (`src/serve/routes/hooks.rs`)
//! against a real bound server: the four-row report, the per-hook toggle, and
//! the read-only 405 that must write nothing to `.git/hooks` (spec AC-33b).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

/// Kills the spawned server on drop so a panicking assertion cannot leak it.
struct ServerGuard(Child);
impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn `comemory serve` on an ephemeral port, returning the base URL, the
/// session token, and the kill-on-drop guard. `extra_args` is appended after
/// `serve --port 0` (e.g. `&["--read-only"]`).
fn spawn_serve(home: &TempDir, extra_args: &[&str]) -> (String, String, ServerGuard) {
    let mut child = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["--json", "serve", "--port", "0"])
        .args(extra_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn serve");
    let stdout = child.stdout.take().expect("piped stdout");
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("read banner");
    let guard = ServerGuard(child);
    let info: serde_json::Value = serde_json::from_str(line.trim()).expect("banner is json");
    let port = info["port"].as_u64().expect("port");
    let token = info["token"].as_str().expect("token").to_string();
    (format!("http://127.0.0.1:{port}"), token, guard)
}

fn build_repo(root: &std::path::Path) -> std::path::PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("fake .git dir");
    repo
}

#[test]
fn v1_get_hooks_reports_three_uninstalled_git_hooks_and_the_config_backed_row() {
    let home = TempDir::new().expect("home");
    let allowed = TempDir::new().expect("allowed dir");
    let repo = build_repo(allowed.path());
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", allowed.path().to_str().expect("utf8 path")],
    );
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/hooks"))
        .header("X-Comemory-Token", &token)
        .query(&[("repo", repo.to_str().expect("utf8 path"))])
        .send()
        .expect("v1 get hooks");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["meta"]["command"], "hooks");
    let hooks = body["data"]["hooks"].as_array().expect("hooks array");
    assert_eq!(hooks.len(), 4);
    assert_eq!(hooks[0]["name"], "post-commit");
    assert_eq!(hooks[0]["installed"], serde_json::json!(false));
    // The fourth row is config-backed, not a file in .git/hooks, and
    // `[reinforce] enabled` defaults to true — so it reads installed on a
    // fresh repo while the three git hooks do not.
    assert_eq!(hooks[3]["name"], "search-edit-reinforcement");
    assert_eq!(hooks[3]["installed"], serde_json::json!(true));
    assert_eq!(hooks[3]["source"], "config");
}

#[test]
fn v1_post_hooks_enable_writes_the_one_requested_hook() {
    let home = TempDir::new().expect("home");
    let allowed = TempDir::new().expect("allowed dir");
    let repo = build_repo(allowed.path());
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", allowed.path().to_str().expect("utf8 path")],
    );
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/hooks"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "repo": repo.to_str().expect("utf8 path"),
            "enable": "post-commit",
        }))
        .send()
        .expect("v1 post hooks enable");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    let hooks = body["data"]["hooks"].as_array().expect("hooks array");
    assert_eq!(hooks[0]["installed"], serde_json::json!(true));
    assert_eq!(hooks[1]["installed"], serde_json::json!(false));
    assert!(repo.join(".git").join("hooks").join("post-commit").exists());
    assert!(!repo.join(".git").join("hooks").join("post-merge").exists());
}

/// AC-33b: `POST /api/v1/hooks` on a `--read-only` server is `405
/// read_only` and writes nothing to `.git/hooks`. Not confirm-gated (module
/// doc) — there is no `confirm` field to omit in the first place, so this
/// test only needs to prove the read-only gate itself.
#[test]
fn v1_post_hooks_on_a_read_only_server_is_405_and_writes_nothing_ac33b() {
    let home = TempDir::new().expect("home");
    let allowed = TempDir::new().expect("allowed dir");
    let repo = build_repo(allowed.path());
    let (base, token, _guard) = spawn_serve(
        &home,
        &[
            "--read-only",
            "--allow-path",
            allowed.path().to_str().expect("utf8 path"),
        ],
    );
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/hooks"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "repo": repo.to_str().expect("utf8 path"),
            "enable": "post-commit",
        }))
        .send()
        .expect("v1 post hooks read-only");
    assert_eq!(res.status().as_u16(), 405);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
    assert!(
        !repo.join(".git").join("hooks").join("post-commit").exists(),
        "a read-only server must write no hook file"
    );
}

#[test]
fn v1_post_hooks_unknown_name_is_a_client_error_and_writes_nothing() {
    let home = TempDir::new().expect("home");
    let allowed = TempDir::new().expect("allowed dir");
    let repo = build_repo(allowed.path());
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", allowed.path().to_str().expect("utf8 path")],
    );
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/hooks"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "repo": repo.to_str().expect("utf8 path"),
            "enable": "pre-push",
        }))
        .send()
        .expect("v1 post hooks unknown");
    assert!(res.status().is_client_error(), "status: {}", res.status());
    assert!(
        !repo.join(".git").join("hooks").join("pre-push").exists(),
        "an unknown hook name must write nothing"
    );
}
