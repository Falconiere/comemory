#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `/api/v1` half of the learned ordering stage (#213), over a real
//! `comemory serve` child process.
//!
//! Two things only the HTTP surface can prove: that the server's shared
//! connection mutex is genuinely released across inference, and that the CLI
//! and HTTP payloads carry the same `learned` object for the same store.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin;
use serde_json::Value;
use tempfile::TempDir;

/// The shipped reference backend, in its deterministic mode.
fn backend_path() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/integrations/reranker/comemory_rerank.py"
    )
}

/// How long the slow scorer stalls before answering.
const SLOW_SECONDS: u64 = 3;

/// A prepared data directory: config written, corpus saved, nothing running.
struct Prepared {
    root: TempDir,
}

impl Prepared {
    /// Seed a store through the real binary and write `config.toml`.
    fn new(rerank: &str) -> Self {
        let root = TempDir::new().expect("tempdir");
        let data_dir = root.path().join(".comemory");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::write(
            data_dir.join("config.toml"),
            format!("[rank]\ndecay = 0.0\n\n{rerank}"),
        )
        .expect("write config.toml");
        let me = Self { root };
        // The same quality spread `tests/learned_rerank.rs` uses: it is what
        // makes the deterministic order and the lexical-overlap order disagree,
        // so a parity assertion over the learned object is not vacuous.
        for (quality, body) in [
            (1, "sqlite busy timeout"),
            (5, "sqlite busy timeout pool retry backoff wal mode"),
            (3, "sqlite busy timeout pool"),
            (3, "sqlite busy timeout pool retry"),
        ] {
            me.save(body, quality);
        }
        me
    }

    fn data_dir(&self) -> PathBuf {
        self.root.path().join(".comemory")
    }

    /// Index two same-named functions in one file, so `search-code` has a
    /// candidate pool whose observation references collide — the shape that
    /// would decline the whole stage if the wire id were the reference string.
    fn seed_code(&self) {
        let conn = comemory::store::connection::open(self.data_dir().join("comemory.db"))
            .expect("open comemory.db");
        for line in [10i64, 40] {
            let id = comemory::store::code_row::insert(
                &conn,
                &comemory::store::code_row::CodeSymbolRow {
                    repo: "demo",
                    path: "alpha.rs",
                    blob_oid: "oid",
                    symbol: "new",
                    kind: "function",
                    lang: "rust",
                    line_start: line,
                    line_end: line + 5,
                    snippet: "fn new() { sqlite busy timeout pool }",
                    simhash: 0,
                    parent_id: None,
                },
            )
            .expect("insert code symbol");
            comemory::store::fts::index_code(
                &conn,
                id,
                "new",
                "fn new() { sqlite busy timeout pool }",
                "alpha.rs",
            )
            .expect("index code fts");
        }
    }

    fn save(&self, body: &str, quality: u8) {
        let quality = quality.to_string();
        let out = Command::new(cargo_bin("comemory"))
            .env("COMEMORY_DATA_DIR", self.data_dir())
            .args([
                "--json",
                "save",
                body,
                "--kind",
                "note",
                "--quality",
                &quality,
            ])
            .output()
            .expect("save");
        assert!(
            out.status.success(),
            "save failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Run `comemory --json <args>` against the same store the server uses.
    fn cli_json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let out = Command::new(cargo_bin("comemory"))
            .env("COMEMORY_DATA_DIR", self.data_dir())
            .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
            .args(&all)
            .output()
            .expect("run comemory");
        assert!(
            out.status.success(),
            "comemory {all:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("not JSON: {e}\n{stdout}"))
    }
}

/// A running `comemory serve` over a prepared store.
struct Server {
    base: String,
    token: String,
    child: Child,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn spawn(prepared: &Prepared) -> Self {
        let mut child = Command::new(cargo_bin("comemory"))
            .env("COMEMORY_DATA_DIR", prepared.data_dir())
            .args(["--json", "serve", "--port", "0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn comemory serve");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("read serve banner");
        let banner: Value = serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("serve banner is not JSON: {e}\n{line}"));
        Self {
            base: format!(
                "http://127.0.0.1:{}",
                banner["port"].as_u64().expect("banner port")
            ),
            token: banner["token"].as_str().expect("banner token").to_string(),
            child,
        }
    }
}

/// One authenticated `GET`, returning the envelope's `data`.
fn get(base: &str, token: &str, path: &str, params: &[(&str, &str)]) -> Value {
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(format!("{base}/api/v1{path}"))
        .header("X-Comemory-Token", token)
        .query(params)
        .send()
        .expect("GET");
    let body: Value = resp.json().expect("envelope");
    assert!(body["ok"] == true, "GET {path} failed: {body}");
    body["data"].clone()
}

/// A scorer that stalls, then executes the real shipped backend.
fn slow_scorer(workspace: &Path) -> PathBuf {
    let script = workspace.join("slow_scorer.py");
    std::fs::write(
        &script,
        format!(
            "#!/usr/bin/env python3\n\
             import subprocess, sys, time\n\
             body = sys.stdin.buffer.read()\n\
             time.sleep({SLOW_SECONDS})\n\
             p = subprocess.run([sys.executable, {backend:?}, 'score', '--scoring', 'lexical-overlap'],\n\
             \x20               input=body, capture_output=True)\n\
             sys.stdout.buffer.write(p.stdout)\n\
             sys.exit(p.returncode)\n",
            backend = backend_path(),
        ),
    )
    .expect("write slow scorer");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = std::fs::metadata(&script).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod");
    }
    script
}

/// The `[rerank]` block pointing at the shipped backend.
fn enabled_block() -> String {
    format!(
        "[rerank]\nenabled = true\ncommand = [\"python3\", {:?}, \"score\", \"--scoring\", \"lexical-overlap\"]\nmodel = \"lexical-overlap@1\"\n",
        backend_path()
    )
}

// ------------------------------------------------------------------ AC-10

#[test]
fn unrelated_read_completes_during_inference() {
    // The store is prepared first so the scorer path can be written beside it.
    let root = TempDir::new().expect("tempdir");
    let data_dir = root.path().join(".comemory");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let scorer = slow_scorer(root.path());
    std::fs::write(
        data_dir.join("config.toml"),
        format!(
            "[rank]\ndecay = 0.0\n\n[rerank]\nenabled = true\ncommand = [{:?}]\nmodel = \"lexical-overlap@1\"\ntimeout_ms = 30000\n",
            scorer.to_string_lossy()
        ),
    )
    .expect("write config.toml");
    let prepared = Prepared { root };
    for (quality, body) in [
        (1, "sqlite busy timeout"),
        (3, "sqlite busy timeout pool"),
        (3, "sqlite busy timeout pool retry"),
    ] {
        prepared.save(body, quality);
    }
    let server = Server::spawn(&prepared);
    let base = server.base.clone();
    let token = server.token.clone();

    let started = Instant::now();
    let slow = std::thread::spawn({
        let (base, token) = (base.clone(), token.clone());
        move || {
            let data = get(&base, &token, "/find", &[("query", "sqlite busy timeout")]);
            (Instant::now(), data)
        }
    });

    // Give the reranked request time to reach the child, then read something
    // that has nothing to do with it.
    std::thread::sleep(Duration::from_millis(400));
    let stats_started = Instant::now();
    let stats = get(&base, &token, "/stats", &[]);
    let stats_done = Instant::now();
    assert!(
        stats["memories"].is_number(),
        "the unrelated read must really have answered with corpus counters: {stats}"
    );
    assert!(
        stats_done.duration_since(stats_started) < Duration::from_secs(1),
        "an unrelated read must not wait on inference (took {:?})",
        stats_done.duration_since(stats_started)
    );

    let (slow_done, data) = slow.join().expect("slow request");
    assert!(
        stats_done < slow_done,
        "the unrelated read must finish while inference is still running"
    );
    assert!(
        slow_done.duration_since(started) >= Duration::from_secs(SLOW_SECONDS),
        "the reranked request really did wait for the scorer"
    );
    assert_eq!(
        data["learned"]["applied"],
        serde_json::json!(true),
        "and it applied the scorer's order: {data}"
    );
    assert!(
        !data["hits"].as_array().expect("hits").is_empty(),
        "the reranked request still returned its page"
    );
}

// ------------------------------------------------------------------ AC-12

#[test]
fn cli_and_http_learned_payload_match() {
    let prepared = Prepared::new(&enabled_block());
    let server = Server::spawn(&prepared);
    let query = "sqlite busy timeout";

    prepared.seed_code();
    for (cli_args, path) in [
        (vec!["search", query], "/memories/search"),
        (vec!["find", query], "/find"),
        (vec!["context", query], "/context"),
        (vec!["search-code", "new"], "/code/search"),
    ] {
        let http_query = cli_args[1];
        let cli = prepared.cli_json(&cli_args);
        let http = get(&server.base, &server.token, path, &[("query", http_query)]);
        let (a, b) = (comparable(&cli["learned"]), comparable(&http["learned"]));
        assert_eq!(
            a, b,
            "{path} and the CLI must report the same learned object\nCLI: {cli}\nHTTP: {http}"
        );
        assert_eq!(
            a["applied"],
            serde_json::json!(true),
            "both surfaces applied the scorer: {a}"
        );
        let scores = a["scores"].as_array().expect("scores");
        assert!(!scores.is_empty(), "{path} scored its candidates: {a}");
        assert!(
            scores
                .iter()
                .all(|s| s["candidate_id"].is_string() && s["candidate_ref"].is_string()),
            "{path} reports both the wire id and the observation reference: {a}"
        );
        // The code fixture is two identical snippets, so their scores tie and
        // the submitted order is preserved by design; the memory surfaces carry
        // the "something actually moved" half of this assertion.
        if path == "/code/search" {
            assert_eq!(
                scores[0]["candidate_ref"], scores[1]["candidate_ref"],
                "the code fixture really does collide on the observation reference: {a}"
            );
            assert_ne!(
                scores[0]["candidate_id"], scores[1]["candidate_id"],
                "while its wire ids do not, which is what keeps the request valid"
            );
        } else {
            assert!(
                scores.iter().any(|s| s["rank"] != s["deterministic_rank"]),
                "at least one candidate must have moved, or parity is vacuous: {a}"
            );
        }
    }
}

/// The learned object with the two fields that legitimately differ per run —
/// the minted request id and the measured wall clock — removed.
fn comparable(v: &Value) -> Value {
    let mut out = v.clone();
    if let Some(obj) = out.as_object_mut() {
        obj.remove("request_id");
        obj.remove("elapsed_ms");
    }
    out
}

#[test]
fn a_declined_scorer_degrades_identically_on_both_surfaces() {
    let prepared = Prepared::new(
        "[rerank]\nenabled = true\ncommand = [\"python3\", \"-c\", \"import sys; sys.stdin.read(); sys.exit(65)\"]\nmodel = \"lexical-overlap@1\"\n",
    );
    let server = Server::spawn(&prepared);
    let query = "sqlite busy timeout";
    let cli = prepared.cli_json(&["search", query]);
    let http = get(
        &server.base,
        &server.token,
        "/memories/search",
        &[("query", query)],
    );
    assert_eq!(cli["learned"]["applied"], serde_json::json!(false));
    assert_eq!(http["learned"]["applied"], serde_json::json!(false));
    assert_eq!(
        comparable(&cli["learned"]),
        comparable(&http["learned"]),
        "a refusal must read the same on both surfaces"
    );
    let cli_ids: Vec<&str> = cli["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .map(|h| h["memory_id"].as_str().expect("id"))
        .collect();
    let http_ids: Vec<&str> = http["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .map(|h| h["memory_id"].as_str().expect("id"))
        .collect();
    assert_eq!(cli_ids, http_ids, "both fall back to the same order");
    assert!(!cli_ids.is_empty(), "and the search still answered");
}

/// The console search (`GET|POST /api/v1/search`) is a fifth payload that
/// funnels into `retrieval::find` and therefore carries `learned` too. It
/// reshapes its hits, so it gets its own case rather than joining the parity
/// loop above.
#[test]
fn the_console_search_carries_the_learned_object() {
    let prepared = Prepared::new(&enabled_block());
    let server = Server::spawn(&prepared);
    let data = get(
        &server.base,
        &server.token,
        "/search",
        &[("q", "sqlite busy timeout")],
    );
    assert_eq!(
        data["learned"]["applied"],
        serde_json::json!(true),
        "the console surface reranks like the others: {data}"
    );
    let scores = data["learned"]["scores"]
        .as_array()
        .expect("scores in the console payload");
    assert!(
        scores.iter().any(|s| s["rank"] != s["deterministic_rank"]),
        "and its order really moved: {data}"
    );
    assert!(
        !data["hits"].as_array().expect("hits").is_empty(),
        "while still returning its reshaped hits: {data}"
    );
}
