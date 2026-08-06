//! End-to-end coverage of `POST /api/v1/mine` and `POST /api/v1/hooks/install`
//! (`src/serve/routes/maint/admin.rs`) against a real bound server: the
//! confirm gate (`hooks/install` only — `mine` carries none per the route
//! table), `--repo` containment to an allowed root, and the read-only 405.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use assert_cmd::Command as AssertCommand;
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

/// Save one memory via the real binary under `home`'s data dir.
fn save_memory(home: &TempDir, body: &str) {
    AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["save", body, "--kind", "note"])
        .assert()
        .success();
}

#[test]
fn v1_mine_reports_without_writing_query_expansions() {
    let home = TempDir::new().expect("home");
    save_memory(&home, "background noise memory unrelated to any query");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/mine"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({}))
        .send()
        .expect("v1 mine");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "mine");
    assert_eq!(body["data"]["applied"], serde_json::json!(false));
}

#[test]
fn v1_mine_on_a_read_only_server_is_405() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/mine"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({}))
        .send()
        .expect("v1 mine read-only");
    assert_eq!(res.status().as_u16(), 405);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
}

#[test]
fn v1_hooks_install_without_confirm_is_400_confirmation_required() {
    // Containment is checked before the confirm gate (§Security), so `repo`
    // must sit inside an allowed root here — otherwise the assertion would
    // conflate the two gates and could observe a 403 instead.
    let home = TempDir::new().expect("home");
    let allowed = TempDir::new().expect("allowed dir");
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", allowed.path().to_str().expect("utf8 path")],
    );
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/hooks/install"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "repo": allowed.path().to_str().expect("utf8 path") }))
        .send()
        .expect("v1 hooks/install no confirm");
    assert_eq!(res.status().as_u16(), 400);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "confirmation_required");
}

#[test]
fn v1_hooks_install_outside_every_allowed_root_is_403_forbidden() {
    let home = TempDir::new().expect("home");
    let outside = TempDir::new().expect("outside dir");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/hooks/install"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "repo": outside.path().to_str().expect("utf8 path"),
            "confirm": true,
        }))
        .send()
        .expect("v1 hooks/install outside root");
    assert_eq!(res.status().as_u16(), 403);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "forbidden");
}

#[test]
fn v1_hooks_install_confirmed_inside_an_allow_path_root_installs_hooks() {
    // A disposable `--allow-path` root — never the checkout this test runs
    // from — so the assertion covers containment without mutating anything
    // outside the tempdir.
    let home = TempDir::new().expect("home");
    let allowed = TempDir::new().expect("allowed dir");
    let repo = allowed.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("fake .git dir");
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", allowed.path().to_str().expect("utf8 path")],
    );
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/hooks/install"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "repo": repo.to_str().expect("utf8 path"),
            "confirm": true,
        }))
        .send()
        .expect("v1 hooks/install confirmed");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "hooks.install");
    assert_eq!(body["data"]["installed"].as_array().map(Vec::len), Some(3));
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        assert!(repo.join(".git").join("hooks").join(hook).exists());
    }
}
