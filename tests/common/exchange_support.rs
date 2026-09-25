#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! Real-process harness for the exchange suites (#255): a hub — a real
//! `comemory serve` over its own data directory, behind a [`FaultProxy`] — and
//! clients — real data directories driven through the real CLI binary, each
//! with an `auth.json` naming the hub through the proxy.
//!
//! Nothing here answers a request. The policy a client uses is written through
//! the same store API a policy load writes (the #253 precedent), because a bare
//! engine has no policy route to load one from.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use serde_json::{Value, json};

use crate::fault_proxy::FaultProxy;
use crate::replica_support::Engine;

/// The workspace every suite's credential names.
pub const WORKSPACE: &str = "ws_exchange";

/// The canonical repository the suites approve.
pub const REPO: &str = "falconiere/comemory";

/// A hub: a real engine over a data directory it can be stopped, restarted,
/// copied and restored under, behind a fault proxy whose origin never changes.
pub struct Hub {
    _root: tempfile::TempDir,
    data: PathBuf,
    engine: Option<Engine>,
    /// The proxy every client talks through.
    pub proxy: FaultProxy,
}

impl Hub {
    /// A fresh hub.
    pub fn start() -> Self {
        Self::start_prepared(|_| {})
    }

    /// A hub whose data directory `prepare` fills before the engine starts —
    /// markdown to rebuild from, a config file.
    pub fn start_prepared(prepare: impl FnOnce(&Path)) -> Self {
        let root = tempfile::tempdir().expect("hub root");
        let data = root.path().join("hub");
        std::fs::create_dir_all(&data).expect("hub data dir");
        prepare(&data);
        let engine = Engine::spawn_at(&data, &[]);
        let proxy = FaultProxy::start(addr_of(&engine));
        Self {
            _root: root,
            data,
            engine: Some(engine),
            proxy,
        }
    }

    /// The `api_url` a client's credential names: the proxy, plus the engine's
    /// `/api` prefix, so `{api_url}/v1/sync/replica/…` reaches the engine route.
    pub fn api_url(&self) -> String {
        format!("{}/api", self.proxy.origin())
    }

    /// The running engine.
    pub fn engine(&self) -> &Engine {
        self.engine.as_ref().expect("hub is running")
    }

    /// The engine's session token — the credential a client presents.
    pub fn token(&self) -> String {
        self.engine().token.clone()
    }

    /// The hub's data directory.
    pub fn data_dir(&self) -> PathBuf {
        self.data.clone()
    }

    /// Stop the engine; the proxy answers `502` until [`Hub::restart`].
    pub fn stop(&mut self) {
        self.engine = None;
        self.proxy.set_upstream(None);
    }

    /// Start the engine again over the same data directory. Its token rotates,
    /// which is how a credential gets revoked for real.
    pub fn restart(&mut self) {
        let engine = Engine::spawn_at(&self.data, &[]);
        self.proxy.set_upstream(Some(addr_of(&engine)));
        self.engine = Some(engine);
    }

    /// Copy the stopped-or-running hub's database files into `to`.
    pub fn snapshot_to(&self, to: &Path) {
        std::fs::create_dir_all(to).expect("snapshot dir");
        let conn = rusqlite::Connection::open(self.data.join("comemory.db")).expect("open hub db");
        conn.execute(
            "VACUUM INTO ?1",
            [to.join("comemory.db").to_str().expect("utf8")],
        )
        .expect("vacuum into snapshot");
        copy_tree(&self.data.join("memories"), &to.join("memories"));
    }

    /// Replace the hub's state with a snapshot taken earlier, then restart —
    /// a restore from backup that keeps the old epoch.
    pub fn restore_from(&mut self, from: &Path) {
        self.stop();
        for name in ["comemory.db", "comemory.db-wal", "comemory.db-shm"] {
            let _ = std::fs::remove_file(self.data.join(name));
        }
        std::fs::copy(from.join("comemory.db"), self.data.join("comemory.db")).expect("restore db");
        let _ = std::fs::remove_dir_all(self.data.join("memories"));
        copy_tree(&from.join("memories"), &self.data.join("memories"));
        self.restart();
    }

    /// Replace the hub with a brand-new engine (a new stream epoch) behind the
    /// same proxy.
    pub fn replace(&mut self) {
        self.stop();
        std::fs::remove_dir_all(&self.data).expect("remove hub data");
        self.restart();
    }

    /// Open the hub's database directly.
    pub fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.data.join("comemory.db")).expect("open hub db")
    }

    /// `(operation_id, entity_key, op)` for every feed position, oldest first.
    pub fn feed(&self) -> Vec<(String, String, String)> {
        let conn = self.db();
        let mut statement = conn
            .prepare("SELECT operation_id, entity_key, op FROM replica_feed ORDER BY sequence")
            .expect("prepare");
        statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect")
    }

    /// Drive the hub's journal seeding to completion — it scans a bounded
    /// batch per replica call — straight to the engine, so the proxy log
    /// stays the client's own.
    pub fn finish_seeding(&self) {
        for _ in 0..500 {
            let (_, manifest) = self.engine().get("/api/v1/sync/replica/manifest");
            if manifest["data"]["capabilities"]
                .as_array()
                .is_some_and(|c| !c.is_empty())
            {
                return;
            }
        }
        panic!("the hub never finished seeding its journal");
    }

    /// The hub's current feed head.
    pub fn head(&self) -> i64 {
        self.db()
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM replica_feed",
                [],
                |r| r.get(0),
            )
            .expect("head")
    }
}

/// A client: a data directory the real CLI runs against.
pub struct Client {
    home: tempfile::TempDir,
    data: PathBuf,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// A client with an empty data directory and no credential.
    pub fn new() -> Self {
        let home = tempfile::tempdir().expect("client home");
        let data = home.path().join(".comemory");
        std::fs::create_dir_all(&data).expect("data dir");
        Self { home, data }
    }

    /// The data directory.
    pub fn data_dir(&self) -> PathBuf {
        self.data.clone()
    }

    /// Write the credential a login would leave, naming `hub` through its proxy.
    pub fn login(&self, hub: &Hub) {
        self.write_auth(&hub.api_url(), &hub.token(), WORKSPACE);
    }

    /// Write `auth.json` exactly as `comemory auth login` persists it.
    pub fn write_auth(&self, api_url: &str, secret: &str, workspace: &str) {
        let auth = json!({
            "version": 2,
            "secret": secret,
            "key_prefix": "cmk_test",
            "api_url": api_url,
            "organization_id": "org_exchange",
            "organization_slug": "exchange",
            "organization_name": "Exchange",
            "workspace_id": workspace,
        });
        std::fs::write(
            self.data.join("auth.json"),
            serde_json::to_vec_pretty(&auth).unwrap(),
        )
        .expect("write auth.json");
    }

    /// Approve `repos` for the key `(api_url, workspace)` at `revision`, through
    /// the store API a policy load uses, and refresh the offline approval map
    /// the document capture reads.
    pub fn approve(&self, api_url: &str, workspace: &str, repos: &[&str], revision: i64) {
        use comemory::store::sync_exchange::ExchangeKey;
        use comemory::store::sync_policy_snapshot::{self, PolicySnapshot};
        let conn = self.open();
        let key = ExchangeKey::new(api_url, workspace);
        sync_policy_snapshot::save(
            &conn,
            &key,
            &PolicySnapshot {
                revision,
                fingerprint: format!("rev-{revision}-{}", repos.join(",")),
                allowlist: repos.iter().map(|r| (*r).to_string()).collect(),
                mappings: BTreeMap::new(),
                loaded_at: "2026-09-24T10:00:00Z".to_string(),
            },
        )
        .expect("save snapshot");
        let resolved: Vec<(String, String)> = repos
            .iter()
            .map(|r| ((*r).to_string(), (*r).to_string()))
            .collect();
        comemory::store::repository_approval::replace_all(&conn, &resolved, "2026-09-24T10:00:00Z")
            .expect("approval map");
    }

    /// Open this client's database directly (migrating it on first use).
    pub fn open(&self) -> rusqlite::Connection {
        comemory::store::connection::open(self.data.join("comemory.db")).expect("open client db")
    }

    /// The command every CLI call starts from: this data directory, a private
    /// `HOME`, no daemon side effects, no credential from the environment.
    fn command(&self, env: &[(&str, &str)]) -> Command {
        let mut cmd = Command::new(cargo_bin("comemory"));
        cmd.env("COMEMORY_DATA_DIR", &self.data)
            .env("HOME", self.home.path())
            .env("COMEMORY_SYNC_DAEMON", "0")
            .env_remove("COMEMORY_API")
            .env_remove("COMEMORY_API_KEY");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd
    }

    /// Run `comemory --json <args>`, asserting success.
    pub fn cli(&self, args: &[&str]) -> Value {
        let (code, stdout, stderr) = self.cli_raw(args, &[]);
        assert_eq!(code, 0, "comemory {args:?} failed: {stderr}");
        serde_json::from_str(&stdout).unwrap_or(Value::Null)
    }

    /// Run `comemory --json <args>` with extra env; `(exit code, stdout, stderr)`.
    pub fn cli_raw(&self, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
        let out = self
            .command(env)
            .arg("--json")
            .args(args)
            .output()
            .expect("run comemory");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// A spawned child running `comemory <args>` (a daemon, a long drain).
    pub fn spawn(&self, args: &[&str], env: &[(&str, &str)]) -> std::process::Child {
        self.command(env)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn comemory")
    }

    /// One manual `comemory sync`.
    pub fn sync(&self) -> Value {
        self.cli(&["sync"])
    }

    /// The `exchange` block of `comemory sync --action status --json`.
    pub fn exchange_status(&self) -> Value {
        self.cli(&["sync", "--action", "status"])["exchange"].clone()
    }

    /// Save a memory under `repo` through the real CLI; returns its id.
    pub fn save(&self, body: &str, repo: &str) -> String {
        let saved = self.cli(&["save", "--kind", "decision", "--repo", repo, body]);
        saved["id"]
            .as_str()
            .or_else(|| saved["memory"]["id"].as_str())
            .unwrap_or_else(|| panic!("save printed no id: {saved}"))
            .to_string()
    }

    /// This client's own `comemory serve`, for HTTP writes.
    pub fn serve(&self) -> Engine {
        Engine::spawn_at(&self.data, &[])
    }

    /// `(operation_id, state, hold_reason)` for every outbox row of the kinds
    /// a test writes, oldest first. The activity event every scoped CLI run
    /// also queues is left out: suite 8 is where events are counted.
    pub fn outbox(&self) -> Vec<(String, String, Option<String>)> {
        let conn = self.open();
        let mut statement = conn
            .prepare(
                "SELECT operation_id, state, hold_reason FROM replica_operation \
                 WHERE entity_kind NOT IN ('feedback_event', 'activity_event') \
                 ORDER BY created_at, rowid",
            )
            .expect("prepare");
        statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect")
    }

    /// Live memory ids in this client's mirror.
    pub fn memory_ids(&self) -> Vec<String> {
        let conn = self.open();
        let mut statement = conn
            .prepare("SELECT id FROM memories WHERE deleted_at IS NULL ORDER BY id")
            .expect("prepare");
        statement
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect")
    }
}

/// Fill `data` with `count` pre-journal memories under `repo`: markdown written
/// by a scratch data directory's real CLI, moved over WITHOUT its database,
/// then replayed by the real `comemory rebuild` — so the memories exist and
/// nothing has journalled them, exactly like a corpus older than the journal.
pub fn prejournal_memories(data: &Path, count: usize, repo: &str) {
    let scratch = Client::new();
    let serve = scratch.serve();
    for n in 0..count {
        let (status, body) = serve.post(
            "/api/v1/memories",
            &json!({"body": guide_body(n), "kind": "decision", "repo": repo}),
        );
        assert!(status < 300, "scratch save {n}: {status} {body}");
    }
    drop(serve);
    copy_tree(&scratch.data_dir().join("memories"), &data.join("memories"));
    let out = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", data)
        .args(["--json", "rebuild"])
        .output()
        .expect("run rebuild");
    assert!(
        out.status.success(),
        "rebuild over the copied markdown: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The engine's socket address, parsed from its base URL.
pub fn addr_of(engine: &Engine) -> SocketAddr {
    engine
        .base
        .trim_start_matches("http://")
        .parse()
        .expect("engine base is host:port")
}

/// A body slice of this repository's own guides, distinct per `n`.
pub fn guide_body(n: usize) -> String {
    let guides = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/guides");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&guides)
        .expect("read docs/guides")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    files.sort();
    let text = std::fs::read_to_string(&files[n % files.len()]).expect("read guide");
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = (n / files.len()) % lines.len().max(1);
    format!(
        "{} (#{n})",
        lines[start..]
            .iter()
            .take(3)
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn copy_tree(from: &Path, to: &Path) {
    if !from.exists() {
        return;
    }
    std::fs::create_dir_all(to).expect("create dir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let path = entry.expect("entry").path();
        let target = to.join(path.file_name().expect("name"));
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            std::fs::copy(&path, &target).expect("copy file");
        }
    }
}
