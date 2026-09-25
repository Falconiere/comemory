#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! Shared fixtures for the `replica_recovery*` suites (#256): building a
//! real legacy corpus with the pinned `v0.43.2` binary and reading it back
//! through this build.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;

/// A body slice of this repository's own guides, distinct per `n` — the same
/// shape [`crate::exchange_support::guide_body`] uses, kept local so this
/// module needs no dependency on the exchange suites.
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
    let text = lines[start..]
        .iter()
        .map(|l| l.trim_start_matches(['-', '#', '*', ' ']))
        .filter(|l| !l.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join(" ");
    format!("Entry #{n}: {text}")
}

/// A real legacy corpus: `count` memories saved through the pinned binary,
/// `trashed` of them then soft-deleted (moved to `.trash/`), each with a
/// distinct real body built from this repository's guides.
pub struct LegacyCorpus {
    pub ids: Vec<String>,
    pub bodies: Vec<String>,
    pub trashed_ids: Vec<String>,
}

impl LegacyCorpus {
    /// The first id/body this corpus never trashed.
    pub fn first_live(&self) -> (&str, &str) {
        let i = self.trashed_ids.len();
        (&self.ids[i], &self.bodies[i])
    }
}

/// Save `count` real memories under `repo` through the legacy binary over
/// `data_dir`, then soft-delete the first `trashed` of them.
pub fn build_legacy_corpus(
    data_dir: &Path,
    repo: &str,
    count: usize,
    trashed: usize,
) -> LegacyCorpus {
    std::fs::create_dir_all(data_dir).expect("create legacy data dir");
    let mut ids = Vec::with_capacity(count);
    let mut bodies = Vec::with_capacity(count);
    for n in 0..count {
        let body = guide_body(n);
        let saved = crate::legacy_engine::run_json(
            data_dir,
            &["save", &body, "--kind", "decision", "--repo", repo],
        );
        ids.push(saved["id"].as_str().expect("saved id").to_string());
        bodies.push(body);
    }
    let trashed_ids: Vec<String> = ids.iter().take(trashed).cloned().collect();
    for id in &trashed_ids {
        crate::legacy_engine::run_json(data_dir, &["delete", id]);
    }
    LegacyCorpus {
        ids,
        bodies,
        trashed_ids,
    }
}

/// A few distinctive words from `body` (skipping the shared `"Entry #n:"`
/// prefix), usable as a `find` query that matches only it.
pub fn distinctive_query(body: &str) -> String {
    body.split_whitespace()
        .skip(2)
        .take(6)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run a real `find` then `feedback --used` through the legacy binary, and a
/// second scoped command, so the corpus carries a retained verdict and at
/// least two retained activity runs (each a real command the legacy build
/// already instruments). Returns the `find`'s `query_id`.
pub fn legacy_verdict_and_runs(
    data_dir: &Path,
    repo: &str,
    memory_id: &str,
    query: &str,
) -> String {
    let found = crate::legacy_engine::run_json(data_dir, &["find", query, "--repo", repo]);
    let query_id = found["query_id"].as_str().expect("query id").to_string();
    crate::legacy_engine::run_json(data_dir, &["feedback", &query_id, "--used", memory_id]);
    let _ = crate::legacy_engine::run_json(data_dir, &["stats"]);
    query_id
}

/// Write the label to canonical map a policy load would leave, directly
/// through the store API — the shape `approve_docs`/`approve` in the other
/// suites use for an engine with no platform to load one from.
pub fn approve(engine: &crate::replica_support::Engine, pairs: &[(&str, &str)]) {
    let conn = engine.db();
    let resolved: Vec<(String, String)> = pairs
        .iter()
        .map(|(l, c)| ((*l).to_string(), (*c).to_string()))
        .collect();
    comemory::store::repository_approval::replace_all(&conn, &resolved, "2026-09-25T10:00:00Z")
        .expect("approve");
}

/// Poll the engine's replica manifest until it advertises at least one
/// capability, or panic — the bootstrap walk must eventually finish.
pub fn wait_for_capabilities(engine: &crate::replica_support::Engine) {
    for _ in 0..2000 {
        let (_, manifest) = engine.get("/api/v1/sync/replica/manifest");
        if manifest["data"]["capabilities"]
            .as_array()
            .is_some_and(|c| !c.is_empty())
        {
            return;
        }
    }
    panic!("the engine never finished seeding its journal");
}

/// Whether the pinned legacy binary can open `data_dir`'s `comemory.db`
/// (a plain `list` — the command the module doc on `doctor` names as one
/// that refuses to swallow a broken open).
pub fn legacy_binary_opens(data_dir: &Path) -> bool {
    crate::legacy_engine::run(data_dir, &["list"])
        .status
        .success()
}

/// One retained `feedback_events` row for `memory_id` under `query_id`, if
/// journalled.
pub fn feedback_event_row(
    data_dir: &Path,
    memory_id: &str,
    query_id: &str,
) -> Option<(String, Option<String>)> {
    let conn = open(data_dir);
    conn.query_row(
        "SELECT verdict, event_id FROM feedback_events WHERE memory_id = ?1 AND query_id = ?2",
        [memory_id, query_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .ok()
}

/// How many `activity_log` rows carry a stamped `event_id` — retained runs
/// the capture walk shared.
pub fn shared_activity_count(data_dir: &Path) -> i64 {
    let conn = open(data_dir);
    conn.query_row(
        "SELECT COUNT(*) FROM activity_log WHERE event_id IS NOT NULL",
        [],
        |r| r.get(0),
    )
    .expect("count activity")
}

/// Every `(entity_kind, entity_key)` pair the replica feed holds, in
/// acceptance order.
pub fn feed_entities(data_dir: &Path) -> Vec<(String, String)> {
    let conn = open(data_dir);
    let mut statement = conn
        .prepare("SELECT entity_kind, entity_key FROM replica_feed ORDER BY sequence")
        .expect("prepare");
    statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

/// Whether the journal already holds a revision for `(kind, key)` — the same
/// idempotency check the seeding walks make.
pub fn has_revision(data_dir: &Path, kind: &str, key: &str) -> bool {
    let conn = open(data_dir);
    comemory::store::replica_read::revision(&conn, kind, key)
        .expect("revision lookup")
        .is_some()
}

/// The `schema_meta` value at `key`, if any.
pub fn schema_meta(data_dir: &Path, key: &str) -> Option<String> {
    let conn = open(data_dir);
    conn.query_row("SELECT value FROM schema_meta WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .ok()
}

/// Open `comemory.db` under `data_dir` read-write, with no migration run.
pub fn open(data_dir: &Path) -> Connection {
    Connection::open(data_dir.join("comemory.db")).expect("open db")
}

/// The `comemory.db.pre-v{from}.bak` snapshots present in `data_dir`.
pub fn pre_migration_snapshots(data_dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(data_dir)
        .expect("read data dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("comemory.db.pre-v") && n.ends_with(".bak"))
        })
        .collect()
}

/// Every live memory id this build's mirror holds.
pub fn live_memory_ids(data_dir: &Path) -> Vec<String> {
    let conn = open(data_dir);
    let mut statement = conn
        .prepare("SELECT id FROM memories WHERE deleted_at IS NULL ORDER BY id")
        .expect("prepare");
    statement
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

/// Run this build's own binary over `data_dir` and parse its JSON stdout,
/// asserting success.
pub fn cli_json(data_dir: &Path, args: &[&str]) -> Value {
    let (code, stdout, stderr) = crate::replica_support::cli_raw(data_dir, args);
    assert_eq!(code, 0, "comemory {args:?} failed: {stderr}");
    serde_json::from_str(stdout.trim()).expect("json stdout")
}
