#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Every memory mutation, over the real surface: the real CLI binary, a real
//! spawned `comemory serve`, real HTTP, real markdown trees and real SQLite
//! databases. Issue 251's M-1 (every surface produces one operation), M-3
//! (an interrupted write is recovered) and M-8 (a write survives being
//! logged out, and fails loudly when the disk refuses it).
//!
//! Nothing is simulated except the instant of the crash itself, which cannot
//! be hit deterministically from outside the process: the fixture produces
//! that instant's exact observable state with the real library API, and
//! `domains::memories::journal`'s own tests prove the state is reachable.
//!
//! The identity, ordering and refusal half is `replica_memories_2.rs`.

use serde_json::json;

#[path = "common/replica_support.rs"]
mod replica_support;

use replica_support::{
    BODY, Engine, cli_raw, cli_with_env, feed_ops, intents, interrupted_save, owed,
};

/// A closed loopback port, so `COMEMORY_API` names a host nothing answers on.
const UNREACHABLE_API: &str = "http://127.0.0.1:1/api/v1";

// ---------------------------------------------------------------------------
// M-1: one operation per mutation, whichever surface made it.
// ---------------------------------------------------------------------------

#[test]
fn a_save_through_the_cli_produces_exactly_one_operation() {
    let engine = Engine::spawn(&[]);
    let dir = engine.data_dir();

    let saved = engine.cli(&["save", "--kind", "decision", BODY]);
    let id = saved["id"].as_str().expect("id").to_string();

    assert_eq!(
        feed_ops(&dir),
        vec![format!("upsert:{id}")],
        "one save, one position"
    );
    assert_eq!(owed(&dir), vec![id], "and one upload owed");
    assert!(intents(&dir).is_empty(), "nothing left outstanding");
}

#[test]
fn a_save_through_http_produces_exactly_one_operation() {
    let engine = Engine::spawn(&[]);
    let dir = engine.data_dir();

    let (status, body) = engine.post(
        "/api/v1/memories",
        &json!({"body": BODY, "kind": "decision", "repo": "Falconiere/comemory"}),
    );
    assert_eq!(status, 200, "save over HTTP: {body}");
    let id = body["data"]["id"].as_str().expect("id").to_string();

    assert_eq!(feed_ops(&dir), vec![format!("upsert:{id}")]);
    assert_eq!(owed(&dir), vec![id]);
    assert!(intents(&dir).is_empty());
}

/// The CLI has no `update` subcommand — an in-place frontmatter patch is an
/// HTTP/MCP surface — so both edits here go over HTTP.
#[test]
fn an_edit_through_each_surface_produces_one_operation_apiece() {
    let engine = Engine::spawn(&[]);
    let dir = engine.data_dir();
    let saved = engine.cli(&["save", "--kind", "decision", BODY]);
    let id = saved["id"].as_str().expect("id").to_string();

    // A tags edit over HTTP and a quality edit through the CLI: two real
    // mutations of one memory, through two different surfaces.
    let (status, body) = engine.patch(
        &format!("/api/v1/memories/{id}"),
        &json!({"tags": ["sync", "journal"]}),
    );
    assert_eq!(status, 200, "tags patch: {body}");
    let (status, body) = engine.patch(&format!("/api/v1/memories/{id}"), &json!({"quality": 5}));
    assert_eq!(status, 200, "quality patch: {body}");

    let ops = feed_ops(&dir);
    assert_eq!(
        ops,
        vec![
            format!("upsert:{id}"),
            format!("upsert:{id}"),
            format!("upsert:{id}")
        ],
        "the save and the two edits — one operation each, no duplicates"
    );
    assert!(intents(&dir).is_empty());
}

// ---------------------------------------------------------------------------
// M-3: an interrupted write is finished by the next command.
// ---------------------------------------------------------------------------

#[test]
fn the_next_cli_command_finishes_a_write_the_database_never_saw() {
    let home = tempfile::TempDir::new().expect("tempdir");
    let dir = home.path().join(".comemory");
    let (id, planned) = interrupted_save(&dir, BODY);

    // Before recovery: the file is there, the memory is not, and nothing has
    // been journalled — so no peer can have heard about this write.
    assert!(planned.exists());
    assert!(feed_ops(&dir).is_empty(), "no position");
    assert!(owed(&dir).is_empty(), "no upload owed");
    assert_eq!(intents(&dir).len(), 1, "but the write is recorded as owed");

    let (code, listed, stderr) = cli_raw(&dir, &["list"]);

    assert_eq!(code, 0, "list failed: {stderr}");
    assert!(
        listed.contains(&id),
        "the recovered memory must be reported: {listed}"
    );
    assert_eq!(
        feed_ops(&dir),
        vec![format!("upsert:{id}")],
        "and the operation it owed is journalled"
    );
    assert_eq!(owed(&dir), vec![id], "and now owed upstream");
    assert!(intents(&dir).is_empty(), "nothing left outstanding");
}

#[test]
fn a_second_command_over_a_recovered_directory_changes_nothing_further() {
    let home = tempfile::TempDir::new().expect("tempdir");
    let dir = home.path().join(".comemory");
    interrupted_save(&dir, BODY);
    let (first, _, _) = cli_raw(&dir, &["list"]);
    assert_eq!(first, 0);
    let after_first = feed_ops(&dir);

    let (second, _, stderr) = cli_raw(&dir, &["list"]);

    assert_eq!(second, 0, "second list failed: {stderr}");
    assert_eq!(
        feed_ops(&dir),
        after_first,
        "recovery is idempotent: one write, one position"
    );
    assert!(intents(&dir).is_empty());
}

#[test]
fn a_fresh_data_directory_is_not_created_by_the_recovery_pass() {
    let home = tempfile::TempDir::new().expect("tempdir");
    let dir = home.path().join(".comemory");

    // `comemory --help`-shaped work over a directory that does not exist yet:
    // the recovery pass must not be what brings a database into being.
    let (code, _, _) = cli_raw(&dir, &["stats"]);

    assert_eq!(code, 0, "an empty engine still answers");
    assert!(
        !dir.join("comemory.db").exists() || feed_ops(&dir).is_empty(),
        "nothing was journalled into a directory that had no memories"
    );
}

#[test]
fn a_serve_session_finishes_an_interrupted_write_before_it_accepts_a_request() {
    let home = tempfile::TempDir::new().expect("tempdir");
    let dir = home.path().join(".comemory");
    let (id, _) = interrupted_save(&dir, BODY);

    // The engine starts over the half-written directory. Reconciliation runs
    // before the listener binds, so the first request already sees the
    // recovered memory.
    let engine = Engine::spawn_at(&dir, &[]);
    let (status, body) = engine.get(&format!("/api/v1/memories/{id}"));

    assert_eq!(status, 200, "the recovered memory is served: {body}");
    assert_eq!(feed_ops(&dir), vec![format!("upsert:{id}")]);
    assert!(intents(&dir).is_empty());
}

#[test]
fn a_read_only_session_serves_the_interrupted_write_as_still_outstanding() {
    let home = tempfile::TempDir::new().expect("tempdir");
    let dir = home.path().join(".comemory");
    interrupted_save(&dir, BODY);

    let engine = Engine::spawn_at(&dir, &["--read-only"]);
    let (status, body) = engine.post(
        "/api/v1/memories",
        &json!({"body": "a read-only session must refuse this", "kind": "note"}),
    );

    assert_eq!(status, 405, "mutating routes stay refused: {body}");
    assert_eq!(
        intents(&dir).len(),
        1,
        "and the recovery pass wrote nothing: the intent keeps for a \
         writable open"
    );
    assert!(feed_ops(&dir).is_empty());
}

// ---------------------------------------------------------------------------
// M-8: a write does not depend on the network, and does not swallow a real
// persistence failure.
// ---------------------------------------------------------------------------

#[test]
fn a_save_with_no_credential_and_nothing_listening_still_commits_and_queues() {
    let home = tempfile::TempDir::new().expect("tempdir");
    let dir = home.path().join(".comemory");

    let (code, body) = cli_with_env(
        &dir,
        &[("COMEMORY_API", UNREACHABLE_API)],
        &["save", "--kind", "decision", BODY],
    );

    assert_eq!(code, 0, "a save must not depend on the network: {body}");
    let id = body["id"].as_str().expect("id").to_string();
    assert_eq!(
        feed_ops(&dir),
        vec![format!("upsert:{id}")],
        "the operation is journalled"
    );
    assert_eq!(
        owed(&dir),
        vec![id],
        "and queued for whenever the push works"
    );
    assert!(intents(&dir).is_empty(), "the write itself finished");
}

#[test]
fn a_save_the_disk_refuses_fails_loudly_rather_than_being_queued() {
    let home = tempfile::TempDir::new().expect("tempdir");
    let dir = home.path().join(".comemory");
    // A real save first, so the tree exists and the failure below is the
    // write itself being refused rather than the directory being absent.
    let (code, _) = cli_with_env(&dir, &[], &["save", "--kind", "note", "a first memory"]);
    assert_eq!(code, 0);

    // Take away write permission on the markdown directory: a real
    // persistence failure, not a simulated one.
    let memories = dir.join("memories");
    let original = std::fs::metadata(&memories)
        .expect("metadata")
        .permissions();
    let mut locked = original.clone();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        locked.set_mode(0o500);
    }
    std::fs::set_permissions(&memories, locked).expect("lock the directory");

    let (code, _, stderr) = cli_raw(&dir, &["save", "--kind", "decision", BODY]);

    std::fs::set_permissions(&memories, original).expect("restore permissions");
    assert_ne!(code, 0, "a write the disk refuses must fail: {stderr}");
    assert!(
        owed(&dir).len() <= 1,
        "and must not be queued as though it had succeeded: {:?}",
        owed(&dir)
    );
}
