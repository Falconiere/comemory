#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The exchange gate and the writer pause around an in-place rebuild (#256,
//! B-3). Every case drives the real binaries — a `comemory serve` taking
//! HTTP writes, a `comemory mcp` session saving, and `comemory rebuild` —
//! over one real data directory, with this test process standing in for a
//! running exchange pass by holding `sync.lock` the way every pass does.
//! The store-health case (B-8) upgrades a data directory the real pinned
//! `v0.43.2` binary wrote, with the directory or the database made
//! read-only, and kills upgrades part-way.

#[path = "common/serve_bin.rs"]
mod serve_bin;

#[path = "common/mcp_bin.rs"]
mod mcp_bin;

#[path = "common/legacy_engine.rs"]
mod legacy_engine;

use std::collections::BTreeSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin;
use mcp_bin::McpHome;
use rusqlite::Connection;
use serde_json::{Value, json};
use serve_bin::ServeHome;

const REPO: &str = "gate-corpus";

/// How long any one rebuild may take before the journey gives up.
const REBUILD_BOUND: Duration = Duration::from_mins(3);

/// What one writer was told about one save.
#[derive(Debug)]
enum Outcome {
    Saved(String),
    Busy,
}

/// Hold `sync.lock` from this process, as a running exchange pass does.
fn hold_exchange_gate(data_dir: &Path) -> File {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data_dir.join("sync.lock"))
        .expect("open sync.lock");
    file.lock().expect("hold sync.lock");
    file
}

fn spawn_rebuild(data_dir: &Path, pause_wait: &str) -> Child {
    Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", data_dir)
        .env("COMEMORY_SYNC_PAUSE_WAIT", pause_wait)
        .arg("rebuild")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn comemory rebuild")
}

/// Wait for `child` within [`REBUILD_BOUND`], returning its exit code and
/// stderr.
fn finish(mut child: Child) -> (Option<i32>, String) {
    let deadline = Instant::now() + REBUILD_BOUND;
    while child.try_wait().expect("try_wait").is_none() {
        assert!(Instant::now() < deadline, "rebuild did not finish in time");
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output().expect("rebuild output");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn open(data_dir: &Path) -> Connection {
    Connection::open(data_dir.join("comemory.db")).expect("open db")
}

fn memory_ids(data_dir: &Path) -> BTreeSet<String> {
    let conn = open(data_dir);
    let mut statement = conn.prepare("SELECT id FROM memories").expect("prepare");
    statement
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}

fn journalled(conn: &Connection, id: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM replica_feed WHERE entity_key = ?1",
        [id],
        |r| r.get::<_, i64>(0),
    )
    .expect("feed lookup")
        > 0
}

/// A valid memory markdown file this server's database has never seen: saved
/// through a throwaway server, then its file taken — so only a rebuild from
/// markdown can bring it into `data_dir`'s database.
struct Planted {
    id: String,
    relative: PathBuf,
    bytes: Vec<u8>,
}

fn plantable(count: usize, tag: &str) -> Vec<Planted> {
    let donor = ServeHome::new();
    let memories = donor.data_dir().join("memories");
    (0..count)
        .map(|i| {
            let saved = donor.post(
                "/memories",
                &json!({
                    "body": format!("planted {tag} {i}: only a rebuild from markdown finds this one"),
                    "kind": "note",
                }),
            );
            let id = saved["id"].as_str().expect("id").to_string();
            let path = find_markdown(&memories, &id);
            Planted {
                id,
                relative: path.strip_prefix(&memories).expect("under memories").into(),
                bytes: std::fs::read(&path).expect("read planted markdown"),
            }
        })
        .collect()
}

fn find_markdown(dir: &Path, id: &str) -> PathBuf {
    for entry in std::fs::read_dir(dir).expect("read memories dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == ".trash") {
                continue;
            }
            if let Some(hit) = try_find(&path, id) {
                return hit;
            }
        } else if is_markdown_of(&path, id) {
            return path;
        }
    }
    panic!("no markdown for {id} under {}", dir.display());
}

fn try_find(dir: &Path, id: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| is_markdown_of(p, id))
}

fn is_markdown_of(path: &Path, id: &str) -> bool {
    path.extension().is_some_and(|e| e == "md")
        && std::fs::read_to_string(path).is_ok_and(|text| text.contains(&format!("id: {id}")))
}

fn plant(data_dir: &Path, planted: &Planted) {
    let target = data_dir.join("memories").join(&planted.relative);
    std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
    std::fs::write(&target, &planted.bytes).expect("plant markdown");
}

/// Save through HTTP until `stop`, recording what the server answered.
fn http_writer(srv: &ServeHome, stop: &AtomicBool) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    let mut i = 0;
    while !stop.load(Ordering::Relaxed) {
        let (status, body) = srv.post_raw(
            "/memories",
            &json!({ "body": format!("http write {i} during the rebuild window"), "kind": "note" }),
        );
        outcomes.push(match (status, body["ok"].as_bool()) {
            (_, Some(true)) => Outcome::Saved(body["data"]["id"].as_str().expect("id").into()),
            (503, _) if body["error"]["code"] == "busy" => Outcome::Busy,
            _ => panic!("an HTTP write must succeed or fail busy, got {status}: {body}"),
        });
        i += 1;
    }
    outcomes
}

/// Save through MCP until `stop`, recording what the tool answered.
async fn mcp_writer(mcp: Arc<McpHome>, stop: Arc<AtomicBool>) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    let mut i = 0;
    while !stop.load(Ordering::Relaxed) {
        let result = mcp
            .call(
                "save",
                json!({ "body": format!("mcp write {i} during the rebuild window"), "repo": REPO }),
            )
            .await;
        let data = result.structured_content.unwrap_or(Value::Null);
        outcomes.push(if result.is_error == Some(true) {
            assert_eq!(
                data["code"], "busy",
                "an MCP save must succeed or fail busy: {data}"
            );
            Outcome::Busy
        } else {
            Outcome::Saved(data["id"].as_str().expect("id").into())
        });
        i += 1;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    outcomes
}

#[test]
fn rebuild_pauses_writers_and_replaces_in_place() {
    // `ServeHome` speaks blocking HTTP, which must never run on (or be
    // dropped by) an async task, so the MCP side gets its own runtime.
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let srv = Arc::new(ServeHome::new());
    let data_dir = srv.data_dir();
    srv.post(
        "/memories",
        &json!({ "body": "the baseline memory before any rebuild", "kind": "note" }),
    );
    let planted = plantable(1, "gate").remove(0);
    plant(&data_dir, &planted);
    let mcp = Arc::new(rt.block_on(McpHome::open(&data_dir, srv.workspace(), &[])));

    // A pass holding the gate past `pause_wait`: the rebuild gives up busy
    // and changes nothing.
    let gate = hold_exchange_gate(&data_dir);
    let before = memory_ids(&data_dir);
    let started = Instant::now();
    let (code, stderr) = finish(spawn_rebuild(&data_dir, "2s"));
    assert_eq!(
        code,
        Some(75),
        "a rebuild behind a held gate is busy: {stderr}"
    );
    assert!(stderr.contains("busy"), "{stderr}");
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "the rebuild must wait out pause_wait before giving up: {:?}",
        started.elapsed()
    );
    assert_eq!(
        memory_ids(&data_dir),
        before,
        "a busy rebuild changed the database"
    );
    assert!(!data_dir.join("comemory.db.pre-rebuild.bak").exists());

    // Writers run the whole time; the rebuild waits for the gate, then
    // rebuilds in place.
    let stop = Arc::new(AtomicBool::new(false));
    let http = {
        let (srv, stop) = (Arc::clone(&srv), Arc::clone(&stop));
        std::thread::spawn(move || http_writer(&srv, &stop))
    };
    let mcp_task = rt.spawn(mcp_writer(Arc::clone(&mcp), Arc::clone(&stop)));
    let mut rebuild = spawn_rebuild(&data_dir, "30s");
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        rebuild.try_wait().expect("try_wait").is_none(),
        "the rebuild must wait while an exchange pass holds the gate"
    );
    assert!(
        !memory_ids(&data_dir).contains(&planted.id),
        "nothing is rebuilt while the gate is held"
    );
    drop(gate);
    let (code, stderr) = finish(rebuild);
    assert_eq!(
        code,
        Some(0),
        "the rebuild succeeds once the gate is free: {stderr}"
    );
    std::thread::sleep(Duration::from_millis(750));
    stop.store(true, Ordering::Relaxed);
    let mut outcomes = http.join().expect("http writer");
    outcomes.extend(rt.block_on(mcp_task).expect("mcp writer"));

    let saved: Vec<&String> = outcomes
        .iter()
        .filter_map(|o| match o {
            Outcome::Saved(id) => Some(id),
            Outcome::Busy => None,
        })
        .collect();
    assert!(!saved.is_empty(), "no writer ever succeeded: {outcomes:?}");
    let ids = memory_ids(&data_dir);
    let conn = open(&data_dir);
    for id in saved {
        assert!(
            ids.contains(id),
            "acknowledged write {id} is missing after the rebuild"
        );
        assert!(
            journalled(&conn, id),
            "acknowledged write {id} lost its journal row"
        );
    }
    assert!(
        ids.contains(&planted.id),
        "the rebuild did not read the planted markdown"
    );

    // The server's and the MCP session's next requests read the rebuilt
    // content through the connections they already hold.
    let shown = srv.get(&format!("/memories/{}", planted.id));
    assert_eq!(shown["id"], json!(planted.id), "{shown}");
    let shown = rt.block_on(mcp.data("show", json!({ "id": planted.id })));
    assert_eq!(shown["id"], json!(planted.id), "{shown}");
}

/// A small xorshift, seeded from the clock: the delays only need to spread
/// over the rebuild, not be reproducible.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn killed_rebuilds_leave_a_valid_store() {
    const RUNS: usize = 20;
    let srv = ServeHome::new();
    let data_dir = srv.data_dir();
    for i in 0..150 {
        srv.post(
            "/memories",
            &json!({ "body": format!("corpus memory {i} the killed rebuild must keep"), "kind": "note" }),
        );
    }
    drop(srv);
    let planted = plantable(RUNS, "kill");

    let started = Instant::now();
    let (code, stderr) = finish(spawn_rebuild(&data_dir, "30s"));
    assert_eq!(code, Some(0), "{stderr}");
    let full = started.elapsed();

    let mut seed = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
            % u128::from(u64::MAX),
    )
    .expect("fits")
        | 1;
    let span = u64::try_from(full.as_millis()).expect("fits").max(1) * 6 / 5;
    // What a completed rebuild reads: every markdown file planted so far,
    // including those an earlier killed run never brought in.
    let mut on_disk = memory_ids(&data_dir);
    for p in &planted {
        let before = memory_ids(&data_dir);
        plant(&data_dir, p);
        on_disk.insert(p.id.clone());
        let mut child = spawn_rebuild(&data_dir, "30s");
        std::thread::sleep(Duration::from_millis(next(&mut seed) % span));
        let _ = child.kill();
        let _ = child.wait();

        let conn = comemory::store::connection::open(data_dir.join("comemory.db"))
            .expect("a killed rebuild left a store that does not open");
        let check: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .expect("integrity_check");
        assert_eq!(check, "ok", "a killed rebuild corrupted the store");
        drop(conn);
        let after = memory_ids(&data_dir);
        assert!(
            after == before || after == on_disk,
            "a killed rebuild left neither the old nor the new state"
        );
    }

    let (code, stderr) = finish(spawn_rebuild(&data_dir, "30s"));
    assert_eq!(
        code,
        Some(0),
        "a rebuild after the kills succeeds: {stderr}"
    );
    let ids = memory_ids(&data_dir);
    assert!(planted.iter().all(|p| ids.contains(&p.id)));
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

/// `comemory.db` and whichever of its `-wal`/`-shm` exist.
fn db_files(data_dir: &Path) -> Vec<PathBuf> {
    ["comemory.db", "comemory.db-wal", "comemory.db-shm"]
        .iter()
        .map(|name| data_dir.join(name))
        .filter(|path| path.exists())
        .collect()
}

fn marker_count(data_dir: &Path) -> i64 {
    comemory::store::readiness::open_probe(&data_dir.join("comemory.db"))
        .expect("probe open")
        .query_row(
            "SELECT COUNT(*) FROM schema_meta WHERE key GLOB '[0-9][0-9][0-9][0-9]_*'",
            [],
            |r| r.get(0),
        )
        .expect("markers")
}

/// Start `comemory serve` on `data_dir` and report how it exited, or `None`
/// when it came up (it is then stopped).
fn serve_exit(data_dir: &Path) -> Option<(i32, String)> {
    let mut child = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", data_dir)
        .args(["--json", "serve", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn comemory serve");
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if child.try_wait().expect("try_wait").is_some() {
            let out = child.wait_with_output().expect("serve output");
            return Some((
                out.status.code().expect("exit code"),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

fn daemon_status(data_dir: &Path) -> Value {
    let out = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", data_dir)
        .env("COMEMORY_SYNC_DAEMON", "0")
        .args(["--json", "sync", "daemon", "status"])
        .output()
        .expect("sync daemon status");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("status json")
}

fn assert_unhealthy(data_dir: &Path, state: &str) {
    let status = daemon_status(data_dir);
    assert_eq!(status["store"], json!(state), "{status}");
    assert_eq!(status["healthy"], json!(false), "{status}");
}

#[test]
fn failed_upgrades_are_never_healthy() {
    let root = tempfile::tempdir().expect("tempdir");
    let data_dir = root.path().join(".comemory");
    let empty = tempfile::tempdir().expect("tempdir");
    let status = daemon_status(empty.path());
    assert_eq!(status["store"], json!("absent"), "{status}");
    assert_eq!(status["healthy"], json!(true), "{status}");
    assert!(
        !empty.path().join("comemory.db").exists(),
        "a probe creates nothing"
    );

    for n in 0..5 {
        legacy_engine::run_json(
            &data_dir,
            &[
                "save",
                &format!("legacy decision {n} before the upgrade"),
                "--kind",
                "decision",
                "--repo",
                REPO,
            ],
        );
    }
    let legacy_markers = marker_count(&data_dir);
    assert_unhealthy(&data_dir, "migration_pending");

    // The data directory read-only: nothing can migrate, nowhere to record
    // the failure — still pending, never healthy.
    set_mode(&data_dir, 0o555);
    let refused = serve_exit(&data_dir);
    let markers = marker_count(&data_dir);
    let status = daemon_status(&data_dir);
    set_mode(&data_dir, 0o755);
    let (code, stderr) = refused.expect("serve must refuse a read-only data directory");
    assert_ne!(code, 0);
    assert!(
        stderr.contains("Permission denied") && stderr.contains("pre-migration snapshot"),
        "an actionable error names the cause: {stderr}"
    );
    assert_eq!(markers, legacy_markers, "a refused upgrade adds no marker");
    assert_eq!(status["store"], json!("migration_pending"), "{status}");
    assert_eq!(status["healthy"], json!(false), "{status}");
    assert!(!data_dir.join("store-health.json").exists());

    // Only the database read-only: the snapshot and the record can be
    // written, the first migration cannot.
    let files = db_files(&data_dir);
    for file in &files {
        set_mode(file, 0o444);
    }
    let refused = serve_exit(&data_dir);
    for file in db_files(&data_dir) {
        set_mode(&file, 0o644);
    }
    let (code, stderr) = refused.expect("serve must refuse a read-only database");
    assert_ne!(code, 0, "{stderr}");
    assert_eq!(marker_count(&data_dir), legacy_markers, "{stderr}");
    let record: Value = serde_json::from_slice(
        &std::fs::read(data_dir.join("store-health.json")).expect("the failure is recorded"),
    )
    .expect("record json");
    assert_eq!(record["state"], json!("migration_failed"), "{record}");
    assert_unhealthy(&data_dir, "migration_failed");

    // Upgrades killed part-way: the next open completes the chain.
    for delay in [20, 60, 120, 250, 500] {
        let mut child = Command::new(cargo_bin("comemory"))
            .env("COMEMORY_DATA_DIR", &data_dir)
            .args(["--json", "serve", "--port", "0"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn comemory serve");
        std::thread::sleep(Duration::from_millis(delay));
        let _ = child.kill();
        let _ = child.wait();
    }
    let srv = ServeHome::spawn_in(root, &[], &[]);
    let status = daemon_status(&srv.data_dir());
    assert_eq!(status["store"], json!("ready"), "{status}");
    assert_eq!(status["healthy"], json!(true), "{status}");
    assert!(
        !srv.data_dir().join("store-health.json").exists(),
        "a successful open clears the record"
    );
    let listed = srv.get("/memories");
    assert!(listed.to_string().contains("legacy decision 0"), "{listed}");
}
