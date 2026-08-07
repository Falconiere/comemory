#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of the `/api/v1` route mount (`src/serve/routes/mod.rs`)
//! against a real bound server: `GET /api/v1/health`'s envelope shape, the
//! path-aware 401 the router `guard` now returns on `/api/v1/*` (AC-11), and
//! that the legacy, unversioned `GET /api/health` keeps its plain-text 401 and
//! bare (unenveloped) payload. Mirrors the `cli__serve.rs` /
//! `serve__search.rs` spawn/banner/authed-request harness.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use assert_cmd::cargo::cargo_bin;
use axum::http::StatusCode;
use comemory::memory::Kind;
use comemory::serve::envelope;
use comemory::serve::routes::{RouteEntry, require_confirm, table};
use tempfile::TempDir;

#[path = "common/serve_state.rs"]
mod serve_state;

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

#[test]
fn v1_health_with_token_returns_the_envelope() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/health"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 health");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("v1 health json");

    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["data"]["read_only"], serde_json::json!(false));
    assert!(
        body["data"]["version"].is_string(),
        "data carries the version string: {body}"
    );
    assert_eq!(body["meta"]["command"], "health");
    assert!(
        body["meta"]["elapsed_ms"].is_u64(),
        "meta carries elapsed_ms: {body}"
    );
}

#[test]
fn v1_health_without_token_is_an_enveloped_401() {
    let home = TempDir::new().expect("home");
    let (base, _token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/health"))
        .send()
        .expect("v1 health no token");
    assert_eq!(res.status().as_u16(), 401);
    let body: serde_json::Value = res
        .json()
        .unwrap_or_else(|e| panic!("a v1 401 body must still be JSON (enveloped, AC-11): {e}"));

    assert_eq!(body["ok"], serde_json::json!(false));
    assert_eq!(body["error"]["code"], "unauthorized");
    assert_eq!(body["meta"]["command"], "auth");
}

#[test]
fn legacy_health_without_token_stays_plain_text_401() {
    let home = TempDir::new().expect("home");
    let (base, _token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    // `/api/health` is gated by the same `guard` (any `/api/*` path), but
    // outside `/api/v1/*` it must keep today's plain-text body — no envelope.
    let res = client
        .get(format!("{base}/api/health"))
        .send()
        .expect("legacy health no token");
    assert_eq!(res.status().as_u16(), 401);
    let body = res.text().expect("legacy 401 body");
    assert_eq!(body, "missing or invalid token");
    assert!(
        serde_json::from_str::<serde_json::Value>(&body).is_err(),
        "legacy 401 body is plain text, not JSON: {body}"
    );
}

#[test]
fn legacy_health_with_token_stays_a_bare_unenveloped_payload() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/health"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("legacy health");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("legacy health json");

    // Bare `{read_only, version}` — no `ok`/`data`/`meta` envelope wrapper.
    assert_eq!(body["read_only"], serde_json::json!(false));
    assert!(body["version"].is_string());
    assert!(body.get("ok").is_none() && body.get("meta").is_none());
}

#[test]
fn require_confirm_true_is_ok() {
    assert!(require_confirm(true).is_ok());
}

#[test]
fn require_confirm_false_maps_to_400_confirmation_required() {
    let err = require_confirm(false).expect_err("false must be gated");
    let (status, code) = envelope::status_and_code(&err);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(code, "confirmation_required");
}

/// A minimal, syntactically-complete JSON body (and, for routes whose
/// extractor needs a required query param, extra `(key, value)` pairs) for
/// one `mutating` route — just enough to satisfy the route's `Json`/`Query`
/// extractor so the request reaches the handler body and its read-only
/// gate, without asserting anything about the operation's actual outcome
/// (AC-4 only cares whether the gate fires). Every `mutating` command must
/// be listed here — an unmapped command panics rather than being silently
/// skipped, so a newly added mutating route is forced to extend this table.
fn minimal_request(entry: &RouteEntry) -> (serde_json::Value, Vec<(&'static str, &'static str)>) {
    match entry.command {
        "save" => (
            serde_json::json!({"body": "AC-4 sweep test memory"}),
            vec![],
        ),
        "feedback" => (serde_json::json!({"query_id": "q-sweep-deadbeef"}), vec![]),
        // An empty body is enough for every command below. `rebuild`, `tune`
        // and `bandit` carry no `confirm`/`apply`, so on a normal server they
        // stop at the confirm gate (`400`) and the sweep never actually
        // rebuilds or retunes — exactly what AC-4 wants, since it only
        // asserts whether the read-only gate fired.
        "delete" | "prune" | "gc" | "mine" | "install-hooks" | "rebuild" | "ingest-code"
        | "tune" | "bandit" => (serde_json::json!({}), vec![]),
        "unindex" => (
            serde_json::json!({}),
            vec![("target", "ac4-sweep-nonexistent")],
        ),
        "index-code" => (
            serde_json::json!({"repo": "sweep", "path": "/nonexistent/ac4-sweep"}),
            vec![],
        ),
        "index" => (
            serde_json::json!({"path": ["/nonexistent/ac4-sweep"]}),
            vec![],
        ),
        other => panic!(
            "minimal_request: no minimal body/query wired for mutating command {other:?} — \
             add one so the AC-4 read-only sweep stays exhaustive"
        ),
    }
}

/// Send one `entry` request against `base`, filling in a dummy `{id}` path
/// segment where the route needs one.
fn send_mutating(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    entry: &RouteEntry,
) -> reqwest::blocking::Response {
    let path = entry.path.replace("{id}", "deadbeef");
    let (body, query) = minimal_request(entry);
    let req = match entry.method {
        "POST" => client.post(format!("{base}/api/v1{path}")),
        "DELETE" => client.delete(format!("{base}/api/v1{path}")),
        other => panic!("send_mutating: unhandled HTTP method {other:?} for {entry:?}"),
    };
    req.header("X-Comemory-Token", token)
        .query(&query)
        .json(&body)
        .send()
        .unwrap_or_else(|e| panic!("request for {entry:?} failed: {e}"))
}

#[test]
fn ac4_every_mutating_route_405s_on_a_read_only_server() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    for entry in table().into_iter().filter(|e| e.mutating) {
        let res = send_mutating(&client, &base, &token, &entry);
        assert_eq!(
            res.status().as_u16(),
            405,
            "{} {} must 405 on a read-only server",
            entry.method,
            entry.path
        );
        let body: serde_json::Value = res.json().unwrap_or_else(|e| {
            panic!(
                "{} {} 405 body must be JSON envelope: {e}",
                entry.method, entry.path
            )
        });
        assert_eq!(
            body["error"]["code"],
            serde_json::json!("read_only"),
            "{} {} body: {body}",
            entry.method,
            entry.path
        );
    }
}

#[test]
fn ac4_no_mutating_route_405s_on_a_normal_server() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    for entry in table().into_iter().filter(|e| e.mutating) {
        let res = send_mutating(&client, &base, &token, &entry);
        assert_ne!(
            res.status().as_u16(),
            405,
            "{} {} must not 405 without --read-only",
            entry.method,
            entry.path
        );
    }
}

#[test]
fn ac4_read_routes_stay_functional_on_a_read_only_server() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    for path in [
        "/api/v1/sources",
        "/api/v1/doctor",
        "/api/v1/memories/search?query=x",
    ] {
        let res = client
            .get(format!("{base}{path}"))
            .header("X-Comemory-Token", &token)
            .send()
            .unwrap_or_else(|e| panic!("read route {path} failed: {e}"));
        assert_ne!(
            res.status().as_u16(),
            405,
            "read route {path} must survive read-only"
        );
    }

    let ast_res = client
        .post(format!("{base}/api/v1/code/ast"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "pattern": "fn $NAME()",
            "lang": "rs",
            "file": "/nonexistent-for-ac4",
        }))
        .send()
        .expect("post /code/ast");
    assert_ne!(
        ast_res.status().as_u16(),
        405,
        "POST /code/ast must survive read-only (it is not `mutating`)"
    );
}

/// In-process (`tower::ServiceExt::oneshot`, no subprocess) sweep of every
/// read-only `/api/v1` GET route this step targets for coverage: each must
/// answer `200` with `ok:true`. Complements the subprocess-based tests
/// above rather than replacing them — this is what actually gets recorded
/// by `cargo llvm-cov` (a SIGKILLed subprocess loses its `.profraw`).
#[tokio::test]
async fn v1_get_routes_all_return_200_ok_envelopes() {
    let session = serve_state::session(false);
    serve_state::save(
        &session,
        "postgres pool exhausted under load spikes",
        Kind::Bug,
        "demo",
    );

    for path in [
        "/api/v1/health",
        "/api/v1/commands",
        "/api/v1/completions?shell=bash",
        "/api/v1/memories",
        "/api/v1/doctor",
        "/api/v1/consolidate",
        "/api/v1/prune",
        "/api/v1/graph",
        "/api/v1/edges?query=postgres",
        "/api/v1/jobs",
        "/api/v1/code/search?query=postgres",
        "/api/v1/memories/search?query=postgres",
        "/api/v1/context?query=postgres",
    ] {
        let resp = serve_state::send(&session, "GET", path, None).await;
        assert_eq!(resp.status, 200, "{path} body: {}", resp.text);
        assert_eq!(
            resp.json["ok"],
            serde_json::json!(true),
            "{path} body: {}",
            resp.text
        );
    }
}

/// `GET /api/v1/commands` (`meta::commands`) — every real clap subcommand
/// mapped onto its `/api/v1` routes; `search` must resolve to a non-empty
/// route list.
#[tokio::test]
async fn v1_commands_maps_search_to_its_http_routes() {
    let session = serve_state::session(false);
    let resp = serve_state::send(&session, "GET", "/api/v1/commands", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    let commands = resp.json["data"]["commands"]
        .as_array()
        .expect("commands array");
    let search = commands
        .iter()
        .find(|c| c["name"] == "search")
        .unwrap_or_else(|| panic!("no `search` entry in {commands:?}"));
    assert_eq!(search["transport"], "http");
    assert!(
        search["routes"].as_array().is_some_and(|r| !r.is_empty()),
        "search: {search}"
    );
}

/// `GET /api/v1/memories/{id}` — 200 for a real id, 404 `not_found` for an
/// unknown one.
#[tokio::test]
async fn v1_memories_get_one_found_and_not_found() {
    let session = serve_state::session(false);
    serve_state::save(
        &session,
        "advisory lock guards the migration runner",
        Kind::Pattern,
        "demo",
    );
    let list = serve_state::send(&session, "GET", "/api/v1/memories", None).await;
    let id = list.json["data"]["items"][0]["id"]
        .as_str()
        .expect("seeded id")
        .to_string();

    let found = serve_state::send(&session, "GET", &format!("/api/v1/memories/{id}"), None).await;
    assert_eq!(found.status, 200, "body: {}", found.text);
    assert_eq!(found.json["data"]["id"], id);
    let content_type = found
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.starts_with("application/json"),
        "content-type: {content_type}"
    );

    let missing = serve_state::send(&session, "GET", "/api/v1/memories/deadbeef", None).await;
    assert_eq!(missing.status, 404, "body: {}", missing.text);
    assert_eq!(missing.json["error"]["code"], "not_found");
}

/// AC-4 (in-process form): `POST /api/v1/memories` on a `--read-only`
/// session is `405 read_only`, not the confirm/validation path.
#[tokio::test]
async fn v1_save_405s_on_a_read_only_session_in_process() {
    let session = serve_state::session(true);
    let resp = serve_state::send(
        &session,
        "POST",
        "/api/v1/memories",
        Some(serde_json::json!({"body": "read-only gate coverage"})),
    )
    .await;
    assert_eq!(resp.status, 405, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], "read_only");
}
