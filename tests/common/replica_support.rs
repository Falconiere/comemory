#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! Real-process fixture for the `replica-v1` contract tests: a spawned
//! `comemory serve` per engine, its data directory, its CLI, and the helpers
//! that build operations from memories the CLI actually saved.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use assert_cmd::cargo::cargo_bin;
use serde_json::{Value, json};

/// Real README prose — representative content, not a fixture string.
pub const BODY: &str = "comemory keeps a durable, searchable memory of the \
decisions, bugs and conventions a codebase accumulates, and links them to the \
code they describe.";

/// Kills the spawned server on drop so a panicking assertion cannot leak it.
struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// One engine: its data directory plus the `serve` process in front of it.
pub struct Engine {
    home: tempfile::TempDir,
    /// The directory this session actually serves — its own temp dir for
    /// [`Engine::spawn`], a caller-supplied one for [`Engine::spawn_at`].
    dir: std::path::PathBuf,
    pub base: String,
    pub token: String,
    _guard: ServerGuard,
}

impl Engine {
    /// Spawn `comemory serve` on an ephemeral port over an EXISTING data
    /// directory, keeping `home` alive for the session's lifetime.
    ///
    /// Lets a case start an engine over a directory it has already put into a
    /// particular state — a half-written memory, say — instead of a fresh one.
    pub fn spawn_at(data_dir: &std::path::Path, extra_args: &[&str]) -> Self {
        let home = tempfile::TempDir::new().expect("tempdir");
        Self::spawn_with(home, data_dir.to_path_buf(), extra_args)
    }

    /// Spawn `comemory serve` on an ephemeral port over a fresh data dir.
    pub fn spawn(extra_args: &[&str]) -> Self {
        let home = tempfile::TempDir::new().expect("tempdir");
        let data_dir = home.path().join(".comemory");
        Self::spawn_with(home, data_dir, extra_args)
    }

    /// Spawn `comemory serve` over `data_dir` with `RUST_LOG=debug` and its
    /// stderr written to `log` — for the cases that must prove a value never
    /// reaches a log line.
    pub fn spawn_logged(data_dir: &std::path::Path, log: &std::path::Path) -> Self {
        let home = tempfile::TempDir::new().expect("tempdir");
        let file = std::fs::File::create(log).expect("create log");
        Self::spawn_inner(home, data_dir.to_path_buf(), &[], Stdio::from(file), true)
    }

    /// The shared spawn: bind port 0, read the JSON banner, keep the child
    /// under a guard that kills it on drop.
    fn spawn_with(
        home: tempfile::TempDir,
        data_dir: std::path::PathBuf,
        extra_args: &[&str],
    ) -> Self {
        Self::spawn_inner(home, data_dir, extra_args, Stdio::null(), false)
    }

    fn spawn_inner(
        home: tempfile::TempDir,
        data_dir: std::path::PathBuf,
        extra_args: &[&str],
        stderr: Stdio,
        debug: bool,
    ) -> Self {
        let mut command = Command::new(cargo_bin("comemory"));
        if debug {
            command.env("RUST_LOG", "debug");
        }
        let mut child = command
            .env("COMEMORY_DATA_DIR", &data_dir)
            .args(["--json", "serve", "--port", "0"])
            .args(extra_args)
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .expect("spawn serve");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("read banner");
        let guard = ServerGuard(child);
        let info: Value = serde_json::from_str(line.trim()).expect("banner is json");
        let port = info["port"].as_u64().expect("port");
        let token = info["token"].as_str().expect("token").to_string();
        Self {
            home,
            dir: data_dir,
            base: format!("http://127.0.0.1:{port}"),
            token,
            _guard: guard,
        }
    }

    /// The data directory this engine serves.
    pub fn data_dir(&self) -> std::path::PathBuf {
        self.dir.clone()
    }

    /// Run a `comemory` subcommand against this engine's data directory.
    pub fn cli(&self, args: &[&str]) -> Value {
        let out = Command::new(cargo_bin("comemory"))
            .env("COMEMORY_DATA_DIR", self.data_dir())
            .arg("--json")
            .args(args)
            .output()
            .expect("run comemory");
        assert!(
            out.status.success(),
            "comemory {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).expect("json stdout")
    }

    /// A read against this engine's API.
    pub fn get(&self, path: &str) -> (u16, Value) {
        ureq_get(&format!("{}{path}", self.base), &self.token)
    }

    /// A write against this engine's API.
    pub fn post(&self, path: &str, body: &Value) -> (u16, Value) {
        ureq_post(&format!("{}{path}", self.base), &self.token, body)
    }

    /// `PATCH` against this session — the frontmatter-patch route's verb.
    pub fn patch(&self, path: &str, body: &Value) -> (u16, Value) {
        ureq_patch(&format!("{}{path}", self.base), &self.token, body)
    }

    /// Stop the server, keeping the data directory — what an operator does
    /// before a rebuild.
    pub fn stop(self) -> tempfile::TempDir {
        self.home
    }

    /// A stopped engine's data directory, usable for CLI commands that need
    /// the database to themselves.
    pub fn reopen(engine: Engine) -> StoppedEngine {
        StoppedEngine {
            home: engine.stop(),
        }
    }

    /// A write the server is expected to refuse before it accepts the body.
    ///
    /// Returns the status when the response arrives, and `None` when the
    /// server closed the connection mid-upload — which is what an early body
    /// rejection looks like to a client that is still writing. Both outcomes
    /// mean refused; which one a run sees is a race, so a test must accept
    /// either or it is flaky.
    pub fn post_expecting_refusal(&self, path: &str, body: &Value) -> Option<u16> {
        match reqwest::blocking::Client::new()
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(body)
            .send()
        {
            Ok(response) => Some(response.status().as_u16()),
            Err(_) => None,
        }
    }

    /// Open this engine's database directly, as an operator would.
    pub fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.data_dir().join("comemory.db")).expect("open db")
    }
}

/// Minimal blocking HTTP GET over the loopback API.
fn ureq_get(url: &str, token: &str) -> (u16, Value) {
    let response = reqwest::blocking::Client::new()
        .get(url)
        .bearer_auth(token)
        .send()
        .expect("GET");
    let status = response.status().as_u16();
    let body: Value = response.json().unwrap_or(Value::Null);
    (status, body)
}

/// Minimal blocking HTTP POST over the loopback API.
fn ureq_patch(url: &str, token: &str, body: &Value) -> (u16, Value) {
    let response = reqwest::blocking::Client::new()
        .patch(url)
        .bearer_auth(token)
        .json(body)
        .send()
        .expect("PATCH");
    let status = response.status().as_u16();
    let parsed: Value = response.json().unwrap_or(Value::Null);
    (status, parsed)
}

fn ureq_post(url: &str, token: &str, body: &Value) -> (u16, Value) {
    let response = reqwest::blocking::Client::new()
        .post(url)
        .bearer_auth(token)
        .json(body)
        .send()
        .expect("POST");
    let status = response.status().as_u16();
    let parsed: Value = response.json().unwrap_or(Value::Null);
    (status, parsed)
}

/// The payload of one memory on `engine`, as a peer would send it.
pub fn payload_of(engine: &Engine, id: &str) -> Value {
    let (status, body) = engine.get("/api/v1/sync/replica/changes?since=0&limit=50");
    assert_eq!(status, 200, "changes: {body}");
    let entries = body["data"]["entries"].as_array().expect("entries");
    let entry = entries
        .iter()
        .find(|e| e["entity_key"] == json!(id))
        .unwrap_or_else(|| panic!("no entry for {id}: {body}"));
    entry["payload"].clone()
}

/// The digest recorded for one memory's newest position on `engine`.
pub fn digest_of(engine: &Engine, id: &str) -> String {
    let (_, body) = engine.get("/api/v1/sync/replica/changes?since=0&limit=50");
    let entries = body["data"]["entries"].as_array().expect("entries").clone();
    entries
        .iter()
        .rev()
        .find(|e| e["entity_key"] == json!(id))
        .and_then(|e| e["payload_digest"].as_str())
        .expect("digest")
        .to_string()
}

/// An upsert envelope carrying `payload` under `operation_id`.
pub fn upsert_envelope(operation_id: &str, id: &str, digest: &str, payload: &Value) -> Value {
    json!({
        "protocol": "replica-v1",
        "operations": [{
            "operation_id": operation_id,
            "entity_kind": "memory",
            "entity_key": id,
            "op": "upsert",
            "schema_version": 1,
            "payload_digest": digest,
            "payload": payload,
        }]
    })
}

/// An engine whose server has been stopped: the data directory alone.
pub struct StoppedEngine {
    home: tempfile::TempDir,
}

impl StoppedEngine {
    /// The data directory.
    pub fn data_dir(&self) -> std::path::PathBuf {
        self.home.path().join(".comemory")
    }

    /// Run a `comemory` subcommand against it.
    pub fn cli(&self, args: &[&str]) -> Value {
        let out = Command::new(cargo_bin("comemory"))
            .env("COMEMORY_DATA_DIR", self.data_dir())
            .arg("--json")
            .args(args)
            .output()
            .expect("run comemory");
        assert!(
            out.status.success(),
            "comemory {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
    }

    /// Open its database directly.
    pub fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.data_dir().join("comemory.db")).expect("open db")
    }
}

// ---------------------------------------------------------------------------
// #251: the memory-mutation surface — the states a killed write leaves, and
// the helpers that build them against a real data directory.
// ---------------------------------------------------------------------------

/// Leave `home` in the state a process killed between the markdown rename and
/// the mirror commit produces: the write intent recorded, the markdown on
/// disk, the database blind to both.
///
/// The kill window cannot be hit deterministically from outside the process,
/// so its exact observable state is produced here with the real library API —
/// the same `MemoryStore::save` the real writer calls, and the same
/// `memory_intent::record`. That the state is genuinely reachable is proven
/// separately by `domains::memories::journal`'s own tests; what these cases
/// prove is that the real CLI recovers from it.
pub fn interrupted_save(data_dir: &std::path::Path, body: &str) -> (String, std::path::PathBuf) {
    use comemory::domains::memories::{Kind, MemoryStore, References, Relations, SaveParams, id};
    use comemory::store::memory_intent::{self, Intent, IntentKind};

    let paths = comemory::config::Paths::new(data_dir);
    paths.ensure_dirs().expect("ensure_dirs");
    let conn = comemory::store::connection::open(paths.db_path()).expect("open db");
    let store = MemoryStore::new(paths);
    let entity_key = id::memory_id(body);
    let planned = store.planned_path(body);
    memory_intent::record(
        &conn,
        &Intent {
            entity_key: entity_key.clone(),
            kind: IntentKind::Write,
            md_path: planned.to_string_lossy().into_owned(),
            operation_id: format!("op-20260922-{}", &entity_key[..8]),
            started_at: "2026-09-22T10:00:00Z".to_string(),
        },
    )
    .expect("record intent");
    store
        .save(SaveParams {
            body,
            kind: Kind::Decision,
            repo: "Falconiere/comemory",
            tags: &["sync".to_string()],
            author: "tester",
            quality: 4,
            relations: Relations::default(),
            references: References::default(),
            created: None,
        })
        .expect("markdown lands");
    (entity_key, planned)
}

/// Every outstanding write intent in `data_dir`.
pub fn intents(data_dir: &std::path::Path) -> Vec<comemory::store::memory_intent::Intent> {
    let paths = comemory::config::Paths::new(data_dir);
    let conn = comemory::store::connection::open(paths.db_path()).expect("open db");
    comemory::store::memory_intent::outstanding(&conn).expect("outstanding")
}

/// Feed positions in `data_dir` as `"<op>:<entity_key>"`, oldest first.
pub fn feed_ops(data_dir: &std::path::Path) -> Vec<String> {
    let paths = comemory::config::Paths::new(data_dir);
    let conn = comemory::store::connection::open(paths.db_path()).expect("open db");
    comemory::store::replica_read::page(&conn, 0, 100, None)
        .expect("page")
        .into_iter()
        .map(|row| format!("{}:{}", row.op.as_str(), row.entity_key))
        .collect()
}

/// Pending outbox entity keys in `data_dir`.
pub fn owed(data_dir: &std::path::Path) -> Vec<String> {
    let paths = comemory::config::Paths::new(data_dir);
    let conn = comemory::store::connection::open(paths.db_path()).expect("open db");
    comemory::store::replica_outbox::pending(&conn, 100)
        .expect("pending")
        .into_iter()
        .map(|row| row.entity_key)
        .collect()
}

/// Answer every outbox row `data_dir` owes, as a successful push would.
///
/// An import is refused while a local change to the same entity is still
/// pending (#251), so a case about the import path drains the outbox first.
pub fn mark_all_pushed(data_dir: &std::path::Path) {
    use comemory::store::replica_outbox::{self, Outcome};
    let paths = comemory::config::Paths::new(data_dir);
    let conn = comemory::store::connection::open(paths.db_path()).expect("open db");
    for row in replica_outbox::pending(&conn, 200).expect("pending") {
        replica_outbox::record(
            &conn,
            &row.operation_id,
            Outcome::Accepted {
                sequence: Some(1),
                disposition: "accepted",
                epoch: None,
            },
            "2026-09-22T10:00:00Z",
        )
        .expect("record the push");
    }
}

/// Run the CLI over `data_dir` and return `(exit code, stdout, stderr)`
/// without asserting success — for the cases where failing loudly is the
/// behavior under test.
pub fn cli_raw(data_dir: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", data_dir)
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

/// Run the CLI over `data_dir` with extra environment, asserting success.
pub fn cli_with_env(
    data_dir: &std::path::Path,
    env: &[(&str, &str)],
    args: &[&str],
) -> (i32, Value) {
    let mut cmd = Command::new(cargo_bin("comemory"));
    cmd.env("COMEMORY_DATA_DIR", data_dir);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let out = cmd.arg("--json").args(args).output().expect("run comemory");
    let code = out.status.code().unwrap_or(-1);
    let body = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    (code, body)
}

/// The payload and digest of the NEWEST position `engine` holds for `id`.
///
/// [`payload_of`] takes the first matching entry, which is the right thing
/// for a case with one revision and the wrong thing after an edit — pairing a
/// stale payload with a fresh digest reads as `rejected_invalid`, which is
/// true but is not what such a case is about.
pub fn latest_revision(engine: &Engine, id: &str) -> (Value, String) {
    let (status, body) = engine.get("/api/v1/sync/replica/changes?since=0&limit=100");
    assert_eq!(status, 200, "changes: {body}");
    let entries = body["data"]["entries"].as_array().expect("entries").clone();
    let entry = entries
        .iter()
        .rev()
        .find(|e| e["entity_key"] == json!(id) && e["payload"].is_object())
        .unwrap_or_else(|| panic!("no payload entry for {id}: {body}"));
    (
        entry["payload"].clone(),
        entry["payload_digest"]
            .as_str()
            .expect("digest")
            .to_string(),
    )
}

// ---------------------------------------------------------------------------
// Code generations over the real surface (issue 252).
// ---------------------------------------------------------------------------

/// The repo label every code case files its checkout under.
pub const CODE_REPO: &str = "Falconiere/comemory";

/// Build a git checkout from a pinned prefix of THIS repository's own Rust
/// sources — real files, real symbols, real import lines — and commit it.
///
/// `count` files are taken in sorted path order from `src/`, so two calls
/// with the same count produce the same tree. Returns the working root.
pub fn pinned_repo(root: &std::path::Path, count: usize) -> std::path::PathBuf {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    collect_rs(&source, &mut files);
    files.sort();
    assert!(
        files.len() >= count,
        "this repository has {} source files, fewer than the {count} asked for",
        files.len()
    );
    let repo = root.join("pinned-repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    for (index, file) in files.into_iter().take(count).enumerate() {
        let body = std::fs::read_to_string(&file).expect("read source");
        // Flattened under a stable name so the tree does not depend on this
        // repository's directory layout, only on its contents.
        std::fs::write(repo.join(format!("file_{index:04}.rs")), body).expect("write");
    }
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "fixture@example.com"]);
    git(&repo, &["config", "user.name", "Fixture"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "pinned snapshot"]);
    repo
}

/// Every `.rs` file under `dir`, recursively.
fn collect_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Run one git command in `repo`, asserting it succeeded.
pub fn git(repo: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The generation this data directory would offer for `CODE_REPO`, as the
/// wire payload a push sends — or `None` when it has nothing to offer.
pub fn planned_generation(data_dir: &std::path::Path, repo: &str) -> Option<Value> {
    let conn = comemory::store::connection::open(data_dir.join("comemory.db")).expect("open db");
    let planned = comemory::domains::code::generation::plan(&conn, repo).expect("plan")?;
    let base = comemory::domains::code::replica_payload::CodeGenerationV1::new(
        "",
        planned.generation.parent_id.as_deref(),
        &planned.generation.head,
        planned.generation.mined_commit.as_deref(),
        &planned.projection,
    );
    let id = base.mint_id().expect("mint");
    let payload = base.with_id(&id);
    let (bytes, _) = payload.canonical().expect("canonical");
    Some(serde_json::from_str(&bytes).expect("json"))
}

/// Record and activate `payload`'s generation locally, as a completed upload
/// does — so the next plan names it as the parent.
pub fn publish_locally(data_dir: &std::path::Path, repo: &str) -> String {
    let conn = comemory::store::connection::open(data_dir.join("comemory.db")).expect("open db");
    let planned = comemory::domains::code::generation::plan(&conn, repo)
        .expect("plan")
        .expect("something to publish");
    let id = planned.generation.generation_id.clone();
    let at = "2026-09-22T10:00:00Z";
    comemory::store::code_generation::record(&conn, &planned.generation, at).expect("record");
    comemory::store::code_generation::activate(&conn, repo, &id, at).expect("activate");
    id
}

/// An import envelope carrying one code generation.
pub fn code_envelope(operation_id: &str, repo: &str, payload: &Value) -> Value {
    let (_, digest) =
        comemory::utilities::canonical_json::bytes_and_digest(payload).expect("digest");
    serde_json::json!({
        "protocol": "replica-v1",
        "operations": [{
            "operation_id": operation_id,
            "entity_kind": "code_generation",
            "entity_key": repo,
            "op": "upsert",
            "schema_version": 1,
            "payload_digest": digest,
            "payload": payload,
        }],
    })
}

/// The active generation's id for `repo`, if this data directory has one.
pub fn active_generation(data_dir: &std::path::Path, repo: &str) -> Option<String> {
    let conn = comemory::store::connection::open(data_dir.join("comemory.db")).expect("open db");
    comemory::store::code_generation::active(&conn, repo)
        .expect("active")
        .map(|g| g.generation_id)
}

/// The paths of the projection `repo`'s active generation published here.
pub fn shared_paths(data_dir: &std::path::Path, repo: &str) -> Vec<String> {
    let conn = comemory::store::connection::open(data_dir.join("comemory.db")).expect("open db");
    let Some(active) = comemory::store::code_generation::active(&conn, repo).expect("active")
    else {
        return Vec::new();
    };
    comemory::store::remote_code::projection(&conn, repo, &active.generation_id)
        .expect("projection")
        .files
        .into_iter()
        .map(|f| f.path)
        .collect()
}

// ---------------------------------------------------------------------------
// Document replication (#253) support.
// ---------------------------------------------------------------------------

/// The repository label the document cases share.
pub const DOC_REPO: &str = "Falconiere/comemory";

/// Copy this repository's own `docs/guides` into `<root>/docs/guides` and
/// return the repository root that contains it.
///
/// Real documentation, not fixtures: the chunk boundaries, headings and links
/// these cases assert on are whatever the shipped extractor produces from the
/// files this repository ships.
pub fn docs_tree(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let repo = root.join(name);
    let guides = repo.join("docs").join("guides");
    std::fs::create_dir_all(&guides).expect("create docs dir");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/guides");
    for entry in std::fs::read_dir(&source).expect("read docs/guides") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "md") {
            let file = path.file_name().expect("file name");
            std::fs::copy(&path, guides.join(file)).expect("copy guide");
        }
    }
    repo
}

/// Approve `DOC_REPO` on `engine` and record `root` as its indexed root.
///
/// A policy load is what writes `repository_approval`, and these engines have
/// no platform to load one from — so the two rows a policy load would leave
/// are written directly, into the same real tables the shipped code reads. The
/// alternative would be a fake platform, which would prove less.
pub fn approve_docs(engine: &Engine, root: &std::path::Path) {
    let conn = engine.db();
    comemory::store::repository_approval::replace_all(
        &conn,
        &[(DOC_REPO.to_string(), DOC_REPO.to_string())],
        "2026-09-23T10:00:00Z",
    )
    .expect("approve");
    conn.execute(
        "INSERT INTO repo_marker (repo, root_path) VALUES (?1, ?2) \
         ON CONFLICT(repo) DO UPDATE SET root_path = excluded.root_path",
        rusqlite::params![
            DOC_REPO,
            std::fs::canonicalize(root)
                .expect("canonicalize root")
                .to_str()
                .expect("utf8 root")
        ],
    )
    .expect("record the indexed root");
}

/// Index `dir` as a document source through the real CLI, under `DOC_REPO`.
pub fn index_docs_cli(engine: &Engine, dir: &std::path::Path) -> Value {
    engine.cli(&[
        "index",
        dir.to_str().expect("utf8 path"),
        "--repo",
        DOC_REPO,
    ])
}

/// Every `(shared_id, path)` this data directory has minted a portable name
/// for, ascending by path.
pub fn shared_names(data_dir: &std::path::Path) -> Vec<(String, String)> {
    let conn = comemory::store::connection::open(data_dir.join("comemory.db")).expect("open db");
    let mut statement = conn
        .prepare("SELECT shared_id, path FROM document_share ORDER BY path")
        .expect("prepare");
    statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect")
}

/// The wire payload this data directory journalled for `shared_id`, as a peer
/// would receive it.
pub fn journalled_revision(data_dir: &std::path::Path, shared_id: &str) -> Value {
    let conn = comemory::store::connection::open(data_dir.join("comemory.db")).expect("open db");
    let bytes: String = conn
        .query_row(
            "SELECT p.bytes FROM replica_feed f JOIN replica_payload p ON p.digest = f.payload_digest \
              WHERE f.entity_kind = 'document_revision' AND f.entity_key = ?1 \
              ORDER BY f.sequence DESC LIMIT 1",
            [shared_id],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("no journalled revision for {shared_id}: {e}"));
    serde_json::from_str(&bytes).expect("payload json")
}

/// Every `(entity_key, op)` a document mutation journalled here, oldest first.
pub fn document_feed(data_dir: &std::path::Path) -> Vec<(String, String)> {
    let conn = comemory::store::connection::open(data_dir.join("comemory.db")).expect("open db");
    let mut statement = conn
        .prepare(
            "SELECT entity_key, op FROM replica_feed \
              WHERE entity_kind = 'document_revision' ORDER BY sequence",
        )
        .expect("prepare");
    statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect")
}

/// An import envelope carrying one document revision.
pub fn document_envelope(operation_id: &str, payload: &Value) -> Value {
    let (_, digest) =
        comemory::utilities::canonical_json::bytes_and_digest(payload).expect("digest");
    serde_json::json!({
        "protocol": "replica-v1",
        "operations": [{
            "operation_id": operation_id,
            "entity_kind": "document_revision",
            "entity_key": payload["shared_id"],
            "op": "upsert",
            "schema_version": 1,
            "payload_digest": digest,
            "payload": payload,
        }],
    })
}
