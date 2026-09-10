#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of `GET /api/v1/memories` and
//! `GET /api/v1/memories/{id}` (`src/serve/routes/memories.rs`) against
//! a real bound server, seeded via real `comemory save` calls. Mirrors the
//! `serve__routes__mod.rs` / `cli__serve.rs` spawn/banner/authed-request
//! harness.

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

/// Save a memory through the real binary so both the markdown file and the
/// SQLite mirror row exist before the server is spawned.
fn save(home: &TempDir, body: &str, kind: &str, repo: &str) -> String {
    let out = AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["--json", "save", body, "--kind", kind, "--repo", repo])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("save --json");
    v["id"].as_str().expect("id").to_string()
}

/// Spawn `comemory serve` on an ephemeral port, returning the base URL, the
/// session token, and the kill-on-drop guard.
fn spawn_serve(home: &TempDir) -> (String, String, ServerGuard) {
    let mut child = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["--json", "serve", "--port", "0"])
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
fn v1_memories_list_returns_a_page_envelope() {
    let home = TempDir::new().expect("home");
    save(&home, "alpha decision one", "decision", "alpha");
    save(&home, "beta bug two", "bug", "beta");
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/memories"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 memories");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("v1 memories json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["data"]["total"], serde_json::json!(2));
    assert_eq!(
        body["data"]["items"].as_array().expect("items").len(),
        2,
        "body: {body}"
    );
    assert_eq!(body["meta"]["command"], "list");
}

#[test]
fn v1_memories_list_filters_by_repo() {
    let home = TempDir::new().expect("home");
    save(&home, "alpha decision one", "decision", "alpha");
    save(&home, "beta bug two", "bug", "beta");
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/memories?repo=alpha"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 memories repo filter");
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["data"]["total"], serde_json::json!(1));
    assert_eq!(body["data"]["items"][0]["repo"], "alpha");
}

#[test]
fn v1_memories_get_returns_the_matching_row() {
    let home = TempDir::new().expect("home");
    let id = save(
        &home,
        "advisory lock guards the migration runner",
        "decision",
        "demo",
    );
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/memories/{id}"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 memories get");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["data"]["id"], id);
    assert_eq!(body["data"]["repo"], "demo");
    assert_eq!(body["data"]["kind"], "decision");
    assert!(
        std::path::Path::new(body["data"]["path"].as_str().expect("path"))
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md")),
        "body: {body}"
    );
    // `show`, not the old synthetic `memories.get`: this route now runs
    // `api::show::run`, and the /api/v1 route table names it `show` so the
    // parity walk sees the subcommand as routed. The envelope's `meta.command`
    // must agree with that table — a client comparing the two (or reading
    // `GET /commands`) would otherwise see a name that matches nothing.
    assert_eq!(body["meta"]["command"], "show");
}

/// AC-9: `GET /api/v1/memories/{id}` (`api::show`) returns exactly the same
/// object `comemory show --json` prints, and that object still carries all
/// seven of today's pre-existing `MemoryDetail` fields.
#[test]
fn v1_memories_get_matches_comemory_show_json() {
    let home = TempDir::new().expect("home");
    let id = save(
        &home,
        "the ranker reads frontmatter, never the body",
        "decision",
        "demo",
    );
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/memories/{id}"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 memories get");
    let envelope: serde_json::Value = res.json().expect("json");
    let via_http = &envelope["data"];

    let out = AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["show", &id, "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let via_cli: serde_json::Value = serde_json::from_str(stdout.trim()).expect("show --json");

    // `activation` is ACT-R `ln(n) - d*ln(days+1)`, recomputed from the clock
    // at read time — it is not stored. The CLI runs as a subprocess a few
    // hundred milliseconds after the HTTP call, so for a just-created memory
    // the two land microseconds apart on `days` and differ in the ~1e-6 range
    // (observed: 0.0 vs -5.787e-6). Comparing the whole object exactly made
    // this test fail whenever the two reads straddled a clock tick.
    //
    // The parity this test exists to prove is that both surfaces answer with
    // the same MEMORY, so `activation` is compared with a tolerance and every
    // other field exactly.
    let http_activation = via_http["activation"].as_f64().expect("http activation");
    let cli_activation = via_cli["activation"].as_f64().expect("cli activation");
    assert!(
        (http_activation - cli_activation).abs() < 1e-3,
        "activation must agree within clock drift: http={http_activation} cli={cli_activation}"
    );
    let mut http_rest = via_http.clone();
    let mut cli_rest = via_cli.clone();
    http_rest["activation"] = serde_json::Value::Null;
    cli_rest["activation"] = serde_json::Value::Null;
    assert_eq!(
        http_rest, cli_rest,
        "HTTP and CLI must answer with the identical object (activation aside)"
    );
    for field in ["id", "kind", "repo", "slug", "tags", "references", "path"] {
        assert!(
            via_http.get(field).is_some(),
            "MemoryDetail field {field:?} missing from GET /memories/{{id}}: {via_http}"
        );
    }
}

#[test]
fn v1_memories_get_unknown_id_is_a_404_not_found() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/memories/deadbeef"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 memories get missing");
    assert_eq!(res.status().as_u16(), 404);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(false));
    assert_eq!(body["error"]["code"], "not_found");
}
