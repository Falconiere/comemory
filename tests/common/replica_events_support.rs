#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! Real-process fixture for the feedback and activity replication cases
//! (#254): a real git checkout holding one of this repository's own source
//! files, the real CLI producing real searches, verdicts and activity rows,
//! and the real `replica-v1` routes moving them between spawned engines.
//!
//! Every read here is SQL against the engine's own database file or a real
//! HTTP response — nothing is constructed in place of what the engine wrote.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::replica_support::{Engine, cli_with_env, git};

/// The canonical repository every case approves its checkout labels under.
pub const CANONICAL: &str = "Falconiere/comemory";

/// The source file the checkout carries, and a symbol it declares.
pub const SOURCE_FILE: &str = "feedback.rs";
/// A function `SOURCE_FILE` declares, which `search-code` finds.
pub const SYMBOL: &str = "upsert_used";

/// A query `BODY` answers lexically.
pub const MEMORY_QUERY: &str = "durable searchable memory decisions";

/// A git checkout at `<root>/<label>` holding this repository's own
/// `src/store/feedback.rs` — real code with real symbols.
pub fn feedback_checkout(root: &Path, label: &str) -> PathBuf {
    let repo = root.join(label);
    std::fs::create_dir_all(&repo).expect("create checkout");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/store/feedback.rs");
    std::fs::copy(source, repo.join(SOURCE_FILE)).expect("copy source");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "fixture@example.com"]);
    git(&repo, &["config", "user.name", "Fixture"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "fixture"]);
    repo
}

/// Write the label → canonical map a policy load would leave. These engines
/// have no platform to load one from, so the rows go into the same real table
/// the shipped code reads (the approach `approve_docs` takes).
pub fn approve(engine: &Engine, pairs: &[(&str, &str)]) {
    let conn = engine.db();
    let resolved: Vec<(String, String)> = pairs
        .iter()
        .map(|(l, c)| ((*l).to_string(), (*c).to_string()))
        .collect();
    comemory::store::repository_approval::replace_all(&conn, &resolved, "2026-09-24T10:00:00Z")
        .expect("approve");
}

/// Index `checkout` under `label` with the real CLI. The command prints
/// nothing on a non-TTY success, so its exit code is the observable.
pub fn index_code(engine: &Engine, label: &str, checkout: &Path) {
    let (code, _, stderr) = crate::replica_support::cli_raw(
        &engine.data_dir(),
        &[
            "index-code",
            "--repo",
            label,
            "--path",
            checkout.to_str().expect("utf8 path"),
        ],
    );
    assert_eq!(code, 0, "index-code failed: {stderr}");
}

/// What one real search round left behind: the `find` query id, the memory it
/// returned, and the code symbol `search-code` returned.
pub struct Scene {
    /// `q-…` id the `find` printed.
    pub query_id: String,
    /// Memory id `find` returned.
    pub memory: String,
    /// `code_symbols` rowid `search-code` returned for [`SYMBOL`].
    pub symbol: i64,
}

/// Save a memory under `label`, then run a scoped `find` and `search-code`.
pub fn scene(engine: &Engine, label: &str, body: &str) -> Scene {
    let saved = engine.cli(&["save", body, "--kind", "decision", "--repo", label]);
    let memory = saved["id"].as_str().expect("saved id").to_string();
    let found = engine.cli(&["find", MEMORY_QUERY, "--repo", label]);
    let query_id = found["query_id"].as_str().expect("query id").to_string();
    assert!(
        found["hits"]
            .as_array()
            .expect("hits")
            .iter()
            .any(|h| h["id"] == json!(memory)),
        "the find returned the saved memory: {found}"
    );
    let code = engine.cli(&["search-code", "upsert used", "--repo", label]);
    let symbol = code["hits"]
        .as_array()
        .expect("code hits")
        .iter()
        .find(|h| h["symbol"] == json!(SYMBOL))
        .and_then(|h| h["symbol_id"].as_i64())
        .unwrap_or_else(|| panic!("search-code found {SYMBOL}: {code}"));
    Scene {
        query_id,
        memory,
        symbol,
    }
}

/// Record one real verdict call through the CLI, optionally as `actor`.
pub fn feedback(engine: &Engine, actor: Option<&str>, scene: &Scene, code: bool) -> Value {
    let symbol = scene.symbol.to_string();
    let mut args = vec!["feedback", scene.query_id.as_str(), "--used", &scene.memory];
    if code {
        args.extend(["--used-code", symbol.as_str()]);
    }
    let env: Vec<(&str, &str)> = actor.map(|a| ("COMEMORY_ACTOR", a)).into_iter().collect();
    let (code, body) = cli_with_env(&engine.data_dir(), &env, &args);
    assert_eq!(code, 0, "feedback failed: {body}");
    body
}

/// This database's device id.
pub fn device_of(data_dir: &Path) -> String {
    open(data_dir)
        .query_row(
            "SELECT device_id FROM replica_device WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .expect("device id")
}

/// Every `feedback_events` row as JSON, oldest first.
pub fn feedback_rows(data_dir: &Path) -> Vec<Value> {
    let conn = open(data_dir);
    let mut statement = conn
        .prepare(
            "SELECT id, event_id, device, surface, actor, at, query_id, memory_id, \
                    target_kind, provenance, verdict FROM feedback_events ORDER BY id",
        )
        .expect("prepare feedback rows");
    statement
        .query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "event_id": r.get::<_, Option<String>>(1)?,
                "device": r.get::<_, Option<String>>(2)?,
                "surface": r.get::<_, Option<String>>(3)?,
                "actor": r.get::<_, Option<String>>(4)?,
                "at": r.get::<_, String>(5)?,
                "query_id": r.get::<_, String>(6)?,
                "memory_id": r.get::<_, String>(7)?,
                "target_kind": r.get::<_, String>(8)?,
                "provenance": r.get::<_, String>(9)?,
                "verdict": r.get::<_, String>(10)?,
            }))
        })
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

/// Every `activity_log` row as JSON, oldest first.
pub fn activity_rows(data_dir: &Path) -> Vec<Value> {
    let conn = open(data_dir);
    let mut statement = conn
        .prepare(
            "SELECT id, event_id, device, at, command, source, actor, repo, summary \
               FROM activity_log ORDER BY id",
        )
        .expect("prepare activity rows");
    statement
        .query_map([], |r| {
            let summary: Option<String> = r.get(8)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "event_id": r.get::<_, Option<String>>(1)?,
                "device": r.get::<_, Option<String>>(2)?,
                "at": r.get::<_, String>(3)?,
                "command": r.get::<_, String>(4)?,
                "source": r.get::<_, String>(5)?,
                "actor": r.get::<_, Option<String>>(6)?,
                "repo": r.get::<_, Option<String>>(7)?,
                "summary": summary.map(|s| serde_json::from_str::<Value>(&s).expect("summary json")),
            }))
        })
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

/// Every feed position of `kind`, with its payload and redaction, oldest first.
pub fn journal(data_dir: &Path, kind: &str) -> Vec<Value> {
    let conn = open(data_dir);
    let mut statement = conn
        .prepare(
            "SELECT f.sequence, f.operation_id, f.entity_key, f.origin, f.at, f.repository, \
                    f.payload_digest, p.bytes, p.redaction \
               FROM replica_feed f LEFT JOIN replica_payload p ON p.digest = f.payload_digest \
              WHERE f.entity_kind = ?1 ORDER BY f.sequence",
        )
        .expect("prepare journal");
    statement
        .query_map([kind], |r| {
            let bytes: Option<String> = r.get(7)?;
            Ok(json!({
                "sequence": r.get::<_, i64>(0)?,
                "operation_id": r.get::<_, String>(1)?,
                "entity_key": r.get::<_, String>(2)?,
                "origin": r.get::<_, String>(3)?,
                "at": r.get::<_, String>(4)?,
                "repository": r.get::<_, Option<String>>(5)?,
                "digest": r.get::<_, Option<String>>(6)?,
                "payload": bytes.map(|b| serde_json::from_str::<Value>(&b).expect("payload json")),
                "redaction": r.get::<_, Option<String>>(8)?,
            }))
        })
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

/// `(memory_id, used, irrelevant)` for every memory counter row.
pub fn memory_counters(data_dir: &Path) -> Vec<(String, i64, i64)> {
    let conn = open(data_dir);
    let mut statement = conn
        .prepare("SELECT memory_id, used_count, irrelevant_count FROM feedback ORDER BY memory_id")
        .expect("prepare");
    statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

/// `(repo, path, symbol, used, irrelevant)` for every code counter row.
pub fn code_counters(data_dir: &Path) -> Vec<(String, String, String, i64, i64)> {
    let conn = open(data_dir);
    let mut statement = conn
        .prepare(
            "SELECT repo, path, symbol, used_count, irrelevant_count FROM code_feedback \
              ORDER BY repo, path, symbol",
        )
        .expect("prepare");
    statement
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

/// Row count of `table`.
pub fn count(data_dir: &Path, table: &str) -> i64 {
    open(data_dir)
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .expect("count")
}

/// Every entry of `engine`'s `changes` feed, paged to the end, optionally
/// restricted to the given kinds.
pub fn changes(engine: &Engine, kinds: &[&str]) -> Vec<Value> {
    let mut since = 0_i64;
    let mut out = Vec::new();
    loop {
        let (status, body) = engine.get(&format!(
            "/api/v1/sync/replica/changes?since={since}&limit=500"
        ));
        assert_eq!(status, 200, "changes: {body}");
        let entries = body["data"]["entries"].as_array().expect("entries").clone();
        let Some(next) = body["data"]["next_sequence"].as_i64() else {
            break;
        };
        out.extend(
            entries
                .into_iter()
                .filter(|e| kinds.is_empty() || kinds.iter().any(|k| e["entity_kind"] == json!(k))),
        );
        since = next;
    }
    out
}

/// A change entry as the operation a relay would offer: the same id, key,
/// digest and bytes, and no payload when the entry carries none.
pub fn operation_of(entry: &Value) -> Value {
    let mut op = json!({
        "operation_id": entry["operation_id"],
        "entity_kind": entry["entity_kind"],
        "entity_key": entry["entity_key"],
        "op": entry["op"],
        "schema_version": entry["schema_version"],
    });
    for key in ["payload_digest", "payload", "repository"] {
        if !entry[key].is_null() {
            op[key] = entry[key].clone();
        }
    }
    op
}

/// Offer `operations` to `to`, 500 per envelope; the concatenated results.
pub fn import(to: &Engine, operations: &[Value]) -> Vec<Value> {
    let mut results = Vec::new();
    for chunk in operations.chunks(500) {
        let (status, body) = to.post(
            "/api/v1/sync/replica/import",
            &json!({ "protocol": "replica-v1", "operations": chunk }),
        );
        assert_eq!(status, 200, "import: {body}");
        results.extend(body["data"]["results"].as_array().expect("results").clone());
    }
    results
}

/// Relay every `kinds` entry of `from`'s feed to `to`.
pub fn transfer(from: &Engine, to: &Engine, kinds: &[&str]) -> Vec<Value> {
    let operations: Vec<Value> = changes(from, kinds).iter().map(operation_of).collect();
    import(to, &operations)
}

/// Re-digest a payload after a case changed it, and return the operation.
pub fn reissued(entry: &Value, payload: &Value) -> Value {
    let (_, digest) =
        comemory::utilities::canonical_json::bytes_and_digest(payload).expect("digest");
    let mut op = operation_of(entry);
    op["payload"] = payload.clone();
    op["payload_digest"] = json!(digest);
    op
}

/// The dispositions of `results`, in order.
pub fn dispositions(results: &[Value]) -> Vec<String> {
    results
        .iter()
        .map(|r| r["disposition"].as_str().expect("disposition").to_string())
        .collect()
}

/// Age every row of `table` matching `filter` to a date far past any
/// retention window — standing in only for the passage of days, as
/// `tests/cli__gc.rs` ages its trash entries.
pub fn age(data_dir: &Path, table: &str, filter: &str) {
    open(data_dir)
        .execute(
            &format!("UPDATE {table} SET at = '2020-01-01T00:00:00Z' WHERE {filter}"),
            [],
        )
        .expect("age rows");
}

/// Two engines with approved checkouts of the same repository under different
/// labels, and a real search round on the first.
pub struct Pair {
    _workspace: tempfile::TempDir,
    /// The engine the searches and verdicts ran on.
    pub a: Engine,
    /// A second engine with its own approved label.
    pub b: Engine,
    /// The search round on `a`.
    pub scene: Scene,
}

pub fn pair() -> Pair {
    let workspace = tempfile::tempdir().expect("workspace");
    let a = Engine::spawn(&[]);
    let b = Engine::spawn(&[]);
    let checkout = feedback_checkout(workspace.path(), "checkout-a");
    approve(&a, &[("checkout-a", CANONICAL)]);
    approve(&b, &[("checkout-b", CANONICAL)]);
    index_code(&a, "checkout-a", &checkout);
    let scene = scene(&a, "checkout-a", crate::replica_support::BODY);
    Pair {
        _workspace: workspace,
        a,
        b,
        scene,
    }
}

fn open(data_dir: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(data_dir.join("comemory.db")).expect("open db")
}
