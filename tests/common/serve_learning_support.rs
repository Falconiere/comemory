//! Shared server-spawn + HTTP helpers for `tests/serve__routes__learning.rs`
//! and `tests/serve__routes__learning_2.rs`.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::Command as AssertCommand;
use assert_cmd::cargo::cargo_bin;
use serde_json::Value;
use tempfile::TempDir;

/// Kills the spawned server on drop so a panicking assertion cannot leak it.
pub struct ServerGuard(Child);
impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn `comemory serve` on an ephemeral port with extra env vars,
/// returning the base URL, the session token, and the kill-on-drop guard.
pub fn spawn_serve_with(
    home: &TempDir,
    extra_args: &[&str],
    envs: &[(&str, &str)],
) -> (String, String, ServerGuard) {
    let mut cmd = Command::new(cargo_bin("comemory"));
    cmd.env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["--json", "serve", "--port", "0"])
        .args(extra_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn serve");
    let stdout = child.stdout.take().expect("piped stdout");
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("read banner");
    let guard = ServerGuard(child);
    let info: Value = serde_json::from_str(line.trim()).expect("banner is json");
    let port = info["port"].as_u64().expect("port");
    let token = info["token"].as_str().expect("token").to_string();
    (format!("http://127.0.0.1:{port}"), token, guard)
}

/// [`spawn_serve_with`] with no extra env vars — the common case.
pub fn spawn_serve(home: &TempDir, extra_args: &[&str]) -> (String, String, ServerGuard) {
    spawn_serve_with(home, extra_args, &[])
}

/// Save one memory (optionally tagged) via the real binary under `home`'s
/// data dir, returning its id.
pub fn save(home: &TempDir, body: &str, tags: &[&str]) -> String {
    let mut args = vec!["--json", "save", body, "--kind", "note"];
    for t in tags {
        args.push("--tags");
        args.push(t);
    }
    let assertion = AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(args)
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: Value = serde_json::from_str(stdout.trim()).expect("parse save JSON");
    v["id"].as_str().expect("save id").to_string()
}

/// `GET /jobs`'s reported total — used to prove a rejected job-creating
/// `POST` created no job at all.
pub fn jobs_total(client: &reqwest::blocking::Client, base: &str, token: &str) -> u64 {
    let res = client
        .get(format!("{base}/api/v1/jobs"))
        .header("X-Comemory-Token", token)
        .send()
        .expect("jobs list");
    let body: Value = res.json().expect("json");
    body["data"]["total"].as_u64().expect("total")
}

/// Poll `GET /jobs/{id}` until it reports a terminal status, returning the
/// envelope's `data` object.
pub fn poll_job_terminal(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    job_id: &str,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let res = client
            .get(format!("{base}/api/v1/jobs/{job_id}"))
            .header("X-Comemory-Token", token)
            .send()
            .expect("poll job");
        let body: Value = res.json().expect("json");
        let data = body["data"].clone();
        if matches!(data["status"].as_str(), Some("done") | Some("error")) {
            return data;
        }
        assert!(
            Instant::now() < deadline,
            "job {job_id} never reached a terminal status"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `POST {base}/api/v1/{path}` with `body`, returning the raw response.
pub fn post(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    path: &str,
    body: &Value,
) -> reqwest::blocking::Response {
    client
        .post(format!("{base}/api/v1/{path}"))
        .header("X-Comemory-Token", token)
        .json(body)
        .send()
        .expect("post")
}
