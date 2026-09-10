#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The login-time first sync (AC-2, AC-3), against the real loopback platform.

use comemory::api::{Ctx, save};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::{connection, sync_state};
use comemory::sync::initial::run_initial_sync;

use crate::test_common as common;
use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

fn save_req(body: &str, repo: &str) -> save::Request {
    save::Request {
        body: body.to_string(),
        title: None,
        kind: Kind::Note,
        repo: repo.into(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    }
}

/// One remote entry the organization already holds, for the pull leg.
fn remote_entry(seq: i64, body: &str) -> serde_json::Value {
    let id = comemory::memory::id::memory_id(body);
    let content_hash = comemory::memory::id::sha256_hex(body.trim_end().as_bytes());
    serde_json::json!({
        "seq": seq,
        "op": "upsert",
        "id": id,
        "content_hash": content_hash,
        "at": "2026-09-06T12:00:00Z",
        "author": "peer",
        "record": {
            "frontmatter": {
                "id": id,
                "kind": "note",
                "repo": "acme/backend",
                "tags": [],
                "created": "2026-09-06T12:00:00Z",
                "quality": 3,
                "schema": 1,
                "content_hash": content_hash,
                "references": {"symbols": [], "files": []},
                "relations": {"supersedes": [], "conflicts_with": [], "derived_from": []}
            },
            "body": body,
            "vector": null
        }
    })
}

#[test]
fn initial_sync_pulls_then_pushes_and_records_the_cursor() {
    let remote_body = "a decision the organization already holds before this machine logged in";
    let platform = SyncPlatformState {
        head_seq: 2,
        changes: serde_json::json!([remote_entry(2, remote_body)]),
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);
    let secret = server.snapshot().secret;

    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut cfg = Config::defaults();
    // Isolate the explicit run from `save`'s detached auto-push.
    cfg.sync.after_save = false;
    let mut conn = connection::open(paths.db_path()).unwrap();

    let local_body = "a local decision this machine offers to the organization";
    let local_id = comemory::memory::id::memory_id(local_body);
    let local_hash = comemory::memory::id::sha256_hex(local_body.trim_end().as_bytes());
    server.update(|st| {
        st.import_results = serde_json::json!([{
            "id": local_id,
            "content_hash": local_hash,
            "status": "accepted",
            "seq": 3
        }]);
    });
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        save::run(&mut ctx, save_req(local_body, "acme/backend"), false, None).unwrap();
    }
    drop(conn);

    let auth = common::auth_fixture::seed_org_auth(
        &paths,
        &server.base,
        &secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );
    let stats = run_initial_sync(&paths, &cfg, &auth).expect("initial sync");

    assert_eq!(stats.pulled, 1, "the remote entry must be adopted");
    assert_eq!(stats.pushed, 1, "the local entry must be offered");

    // A count alone would pass on any entry. Prove it is *this* one, on disk.
    let remote_id = comemory::memory::id::memory_id(remote_body);
    let adopted = comemory::memory::MemoryStore::new(paths.clone())
        .load(&remote_id)
        .expect("the pulled memory must be readable from the markdown store");
    assert_eq!(adopted.body.trim_end(), remote_body);
    assert_eq!(adopted.frontmatter.repo, "acme/backend");

    // Order matters: pushing first would let sync_binding claim a memory the
    // organization already holds.
    let paths_seen = server.paths();
    let first_changes = paths_seen
        .iter()
        .position(|p| p == "/v1/sync/changes")
        .expect("a pull happened");
    let first_import = paths_seen
        .iter()
        .position(|p| p == "/v1/sync/import")
        .expect("a push happened");
    assert!(
        first_changes < first_import,
        "pull must precede push, saw: {paths_seen:?}"
    );

    let conn = connection::open(paths.db_path()).unwrap();
    let row = sync_state::get(&conn, &auth.workspace_id)
        .unwrap()
        .expect("a cursor row for the org workspace");
    let stamped = row
        .last_sync_at
        .expect("a successful first sync must stamp last_sync_at");
    let parsed = time::OffsetDateTime::parse(
        &stamped,
        &time::format_description::well_known::Iso8601::DEFAULT,
    )
    .expect("last_sync_at must be ISO-8601");
    let age = (time::OffsetDateTime::now_utc() - parsed).whole_seconds();
    assert!(
        (0..60).contains(&age),
        "the stamp must be from this run, not carried over: {stamped} ({age}s old)"
    );
}

#[test]
fn initial_sync_on_an_empty_pair_succeeds_with_zero_counts() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;

    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let auth = common::auth_fixture::seed_org_auth(
        &paths,
        &server.base,
        &secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );

    let stats = run_initial_sync(&paths, &cfg, &auth).expect("empty first sync still succeeds");
    assert_eq!(stats.pulled, 0);
    assert_eq!(stats.pushed, 0);
}

#[test]
fn initial_sync_surfaces_a_platform_outage_to_its_caller() {
    // The error is raised here and swallowed by the login path, not swallowed
    // here — otherwise `comemory sync` could not report the same failure.
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    server.update(|st| st.sync_unavailable = true);
    let secret = server.snapshot().secret;

    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let auth = common::auth_fixture::seed_org_auth(
        &paths,
        &server.base,
        &secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );

    let err = run_initial_sync(&paths, &cfg, &auth).expect_err("a 500 must surface");
    assert!(err.to_string().contains("500"), "got: {err}");
}
