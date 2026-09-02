#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! Shared real-binary `comemory serve` runner for crate-root HTTP journey
//! tests (`tests/serve_scenario_*.rs`) — the `/api/v1` twin of
//! `cli_bin.rs`.
//!
//! Spawns the real binary on an ephemeral loopback port under a throwaway
//! `COMEMORY_DATA_DIR` (`<temp>/.comemory`, the production layout), reads
//! the `--json` startup banner for `port` + `token`, and kills the child on
//! drop. Every helper unwraps the `{ok, data, meta}` envelope and panics
//! with the full body on `ok: false`, so a journey reads like the CLI one.
//! `dead_code` is allowed because each journey uses its own subset.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin;
use reqwest::blocking::{Client, RequestBuilder, Response};
use serde_json::Value;
use tempfile::TempDir;

/// How long a background job may take before the journey gives up.
const JOB_TIMEOUT: Duration = Duration::from_mins(2);

/// A running `comemory serve` over a throwaway data directory.
pub struct ServeHome {
    root: TempDir,
    base: String,
    token: String,
    client: Client,
    child: Child,
}

impl Drop for ServeHome {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl ServeHome {
    /// Spawn `comemory --json serve --port 0` on a fresh tempdir.
    pub fn new() -> Self {
        Self::with_args(&[])
    }

    /// Like [`ServeHome::new`] with extra `serve` flags (e.g.
    /// `&["--read-only"]`) and extra environment variables.
    pub fn with_args(extra: &[&str]) -> Self {
        Self::with_args_env(extra, &[])
    }

    /// The fully parameterized spawn on a fresh tempdir.
    pub fn with_args_env(extra: &[&str], envs: &[(&str, &str)]) -> Self {
        Self::spawn_in(TempDir::new().expect("tempdir"), extra, envs)
    }

    /// Spawn over a caller-prepared `root` — for a journey that must seed
    /// `<root>/.comemory/config.toml` (or fixtures) before the server
    /// loads its config at startup.
    pub fn spawn_in(root: TempDir, extra: &[&str], envs: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(cargo_bin("comemory"));
        cmd.env("COMEMORY_DATA_DIR", root.path().join(".comemory"))
            .args(["--json", "serve", "--port", "0"])
            .args(extra)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn comemory serve");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("read serve banner");
        let banner: Value = serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("serve banner is not JSON: {e}\n{line}"));
        let port = banner["port"].as_u64().expect("banner port");
        let token = banner["token"].as_str().expect("banner token").to_string();
        Self {
            root,
            base: format!("http://127.0.0.1:{port}"),
            token,
            client: Client::new(),
            child,
        }
    }

    /// `<root>/.comemory` — the server's `COMEMORY_DATA_DIR`.
    pub fn data_dir(&self) -> PathBuf {
        self.root.path().join(".comemory")
    }

    /// `<root>` — where a journey drops git or document fixtures. Sits
    /// beside the data dir, never inside it.
    pub fn workspace(&self) -> &Path {
        self.root.path()
    }

    /// The session token, for a journey that needs a raw request.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Absolute URL for an `/api/v1`-relative path.
    pub fn url(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.base)
    }

    fn authed(&self, rb: RequestBuilder) -> RequestBuilder {
        rb.header("X-Comemory-Token", &self.token)
    }

    fn finish(resp: Response) -> (u16, Value) {
        let status = resp.status().as_u16();
        let text = resp.text().expect("response body");
        let json = serde_json::from_str(text.trim()).unwrap_or(Value::Null);
        (status, json)
    }

    fn data(method: &str, path: &str, (status, body): (u16, Value)) -> Value {
        assert!(
            body["ok"] == true,
            "{method} {path} failed with HTTP {status}: {body}"
        );
        body["data"].clone()
    }

    /// `GET` returning `(status, envelope)` — for a journey asserting an
    /// error shape.
    pub fn get_raw(&self, path: &str) -> (u16, Value) {
        let resp = self
            .authed(self.client.get(self.url(path)))
            .send()
            .expect("GET");
        Self::finish(resp)
    }

    /// `GET` that must succeed; returns the envelope's `data`.
    pub fn get(&self, path: &str) -> Value {
        Self::data("GET", path, self.get_raw(path))
    }

    /// `GET` with URL-encoded query parameters, returning `(status, envelope)`.
    pub fn get_q_raw(&self, path: &str, params: &[(&str, &str)]) -> (u16, Value) {
        let resp = self
            .authed(self.client.get(self.url(path)))
            .query(params)
            .send()
            .expect("GET with query");
        Self::finish(resp)
    }

    /// `GET` with query parameters that must succeed; returns `data`.
    pub fn get_q(&self, path: &str, params: &[(&str, &str)]) -> Value {
        Self::data("GET", path, self.get_q_raw(path, params))
    }

    /// `POST` a JSON body returning `(status, envelope)`.
    pub fn post_raw(&self, path: &str, body: &Value) -> (u16, Value) {
        let resp = self
            .authed(self.client.post(self.url(path)))
            .json(body)
            .send()
            .expect("POST");
        Self::finish(resp)
    }

    /// `POST` a JSON body that must succeed; returns `data`.
    pub fn post(&self, path: &str, body: &Value) -> Value {
        Self::data("POST", path, self.post_raw(path, body))
    }

    /// `POST` a raw text body (NDJSON for `/code/ingest`).
    pub fn post_text_raw(&self, path: &str, body: String) -> (u16, Value) {
        let resp = self
            .authed(self.client.post(self.url(path)))
            .body(body)
            .send()
            .expect("POST text");
        Self::finish(resp)
    }

    /// `PUT` a JSON body that must succeed; returns `data`.
    pub fn put(&self, path: &str, body: &Value) -> Value {
        let resp = self
            .authed(self.client.put(self.url(path)))
            .json(body)
            .send()
            .expect("PUT");
        Self::data("PUT", path, Self::finish(resp))
    }

    /// `PATCH` a JSON body that must succeed; returns `data`.
    pub fn patch(&self, path: &str, body: &Value) -> Value {
        let resp = self
            .authed(self.client.patch(self.url(path)))
            .json(body)
            .send()
            .expect("PATCH");
        Self::data("PATCH", path, Self::finish(resp))
    }

    /// `DELETE` returning `(status, envelope)`.
    pub fn delete_raw(&self, path: &str) -> (u16, Value) {
        let resp = self
            .authed(self.client.delete(self.url(path)))
            .send()
            .expect("DELETE");
        Self::finish(resp)
    }

    /// `DELETE` that must succeed; returns `data`.
    pub fn delete(&self, path: &str) -> Value {
        Self::data("DELETE", path, self.delete_raw(path))
    }

    /// Poll `GET /jobs/{id}` until the job is terminal. Returns the job's
    /// `result`; panics on `error` / `cancelled` or after [`JOB_TIMEOUT`].
    pub fn wait_job(&self, job_id: &str) -> Value {
        let deadline = Instant::now() + JOB_TIMEOUT;
        loop {
            let job = self.get(&format!("/jobs/{job_id}"));
            match job["status"].as_str().unwrap_or("") {
                "done" => return job["result"].clone(),
                "error" | "cancelled" => panic!("job {job_id} did not finish cleanly: {job}"),
                _ => {}
            }
            assert!(
                Instant::now() < deadline,
                "job {job_id} still {} after {JOB_TIMEOUT:?}: {job}",
                job["status"]
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// `POST` a job-creating route (must answer `202` + `job_id`), wait for
    /// it, and return the job's `result`.
    pub fn job(&self, path: &str, body: &Value) -> Value {
        let (status, env) = self.post_raw(path, body);
        assert_eq!(status, 202, "POST {path} must be accepted as a job: {env}");
        let id = env["data"]["job_id"]
            .as_str()
            .unwrap_or_else(|| panic!("POST {path} carries no job_id: {env}"))
            .to_string();
        self.wait_job(&id)
    }

    /// [`ServeHome::job`] for a raw-text (NDJSON) body.
    pub fn job_text(&self, path: &str, body: String) -> Value {
        let (status, env) = self.post_text_raw(path, body);
        assert_eq!(status, 202, "POST {path} must be accepted as a job: {env}");
        let id = env["data"]["job_id"]
            .as_str()
            .unwrap_or_else(|| panic!("POST {path} carries no job_id: {env}"))
            .to_string();
        self.wait_job(&id)
    }
}
