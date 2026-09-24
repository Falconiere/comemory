#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::domains::sync::manual`] — what one `comemory sync` run
//! does, against a real loopback platform and a real SQLite store.

use comemory::config::{Config, Paths};
use comemory::domains::memories::{Kind, save};
use comemory::domains::sync::manual;
use comemory::store::connection;
use comemory::utilities::context::Ctx;

use crate::test_common as common;
use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

fn save_req(body: &str) -> save::Request {
    save::Request {
        body: body.to_string(),
        title: None,
        kind: Kind::Note,
        repo: "falconiere/comemory".into(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    }
}

/// A data dir with one saved memory and an org credential for `server`.
fn seeded(
    server: &SyncPlatformServer,
    secret: &str,
    body: &str,
) -> (tempfile::TempDir, Paths, Config) {
    let mut cfg = Config::defaults();
    // These tests drive the run sequences directly, not the inline push.
    cfg.sync.after_save = false;
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    common::auth_fixture::seed_org_auth(
        &paths,
        &server.base,
        secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );
    let mut conn = connection::open(paths.db_path()).expect("db");
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    save::run(&mut ctx, save_req(body), false, None).expect("save");
    drop(ctx);
    drop(conn);
    (home, paths, cfg)
}

#[test]
fn open_session_refuses_a_machine_that_is_not_logged_in() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let cfg = Config::defaults();

    // The credential is checked before the store is opened, so the refusal is
    // the operator-facing usage error and not a database error.
    let Err(err) = manual::open_session(&paths, &cfg) else {
        panic!("a machine with no credential must not open a sync session");
    };
    assert!(
        matches!(&err, comemory::errors::Error::Usage(m)
            if m == "not logged in — run `comemory auth login`"),
        "unexpected error: {err:?}"
    );
    assert!(
        !paths.db_path().exists(),
        "refusing a logged-out sync must not create the database"
    );
}

#[test]
fn open_session_initializes_an_auth_only_data_directory() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path().join("receiver"));
    common::auth_fixture::seed_org_auth(
        &paths,
        &server.base,
        &secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );

    let _session = manual::open_session(&paths, &Config::defaults()).expect("session");

    assert!(paths.memories_dir().is_dir());
    assert!(paths.index_dir().is_dir());
    assert!(paths.db_path().is_file());
}

#[test]
fn a_full_run_pulls_before_it_pushes_and_always_offers_the_code_index() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;
    let body = "a decision worth syncing";
    let id = comemory::domains::memories::id::memory_id(body);
    let content_hash = comemory::utilities::digest::sha256_hex(body.trim_end().as_bytes());
    server.update(|st| {
        st.import_results = serde_json::json!([{
            "id": id, "content_hash": content_hash, "status": "accepted", "seq": 1
        }]);
    });
    let (_home, paths, cfg) = seeded(&server, &secret, body);
    let mut session = manual::open_session(&paths, &cfg).expect("session");

    let stats = manual::run_all(&paths, &cfg, &mut session, None, manual::RUN_LIMIT).expect("run");

    // All three legs ran, and the report can tell each apart. The fixture
    // serves no remote changes and nothing here is code-indexed, so the pull
    // and code legs are present with zero counts — which is exactly what
    // distinguishes them from a leg that did not run at all.
    assert_eq!(stats.pull.as_ref().expect("pull leg").pulled, 0);
    assert_eq!(stats.push.as_ref().expect("push leg").pushed, 1);
    let code = stats.code.as_ref().expect("code leg");
    assert_eq!((code.repos, code.files_pushed, code.failed), (0, 0, 0));

    // Pull is first on the wire: the platform saw `changes` before `import`.
    let paths_seen: Vec<String> = server.requests().into_iter().map(|r| r.path).collect();
    let changes = paths_seen
        .iter()
        .position(|p| p.starts_with("/v1/sync/changes"));
    let import = paths_seen
        .iter()
        .position(|p| p.starts_with("/v1/sync/import"));
    assert!(
        matches!((changes, import), (Some(c), Some(i)) if c < i),
        "expected a pull before the push, saw {paths_seen:?}"
    );
}

#[test]
fn a_pull_only_run_reports_no_push_and_no_code_leg() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;
    let (_home, paths, cfg) = seeded(&server, &secret, "a note that stays local this run");
    let mut session = manual::open_session(&paths, &cfg).expect("session");

    let stats = manual::pull_only(&paths, &cfg, &mut session, manual::RUN_LIMIT).expect("pull");

    // "Did not run" is distinguishable from "ran and moved nothing": the
    // report's push and code fields are absent rather than zeroed.
    assert_eq!(stats.pull.as_ref().expect("pull leg").pulled, 0);
    assert!(stats.push.is_none());
    assert!(stats.code.is_none());
    assert!(
        !server.saw_path("/v1/sync/import"),
        "a pull-only run must not push"
    );
}

/// A full run re-indexes a hooked repo whose HEAD moved with no hook firing,
/// before its code push, so the push carries the new head.
#[test]
fn a_full_run_refreshes_a_stale_hooked_repo_before_its_code_push() {
    use crate::test_common::code_sync_fixture as fixture;

    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;
    let (home, paths, cfg) = seeded(&server, &secret, "a note riding along");
    let tree = fixture::write_ts_repo(home.path());
    fixture::install_inert_hooks(&paths, &cfg, &tree);
    let mut conn = connection::open(paths.db_path()).expect("db");
    fixture::index(&paths, &cfg, &mut conn, &tree);
    drop(conn);
    let head = fixture::commit_hookless(
        &tree,
        &[(
            "src/a.ts",
            "export function alpha(): number {\n  return 7;\n}\n",
        )],
        "touch a",
    );
    let mut session = manual::open_session(&paths, &cfg).expect("session");

    let stats = manual::run_all(&paths, &cfg, &mut session, None, manual::RUN_LIMIT).expect("run");

    let refresh = stats.refresh.as_ref().expect("run refreshes");
    assert_eq!(
        (refresh.checked, refresh.refreshed, refresh.failed),
        (1, 1, 0)
    );
    assert_eq!(stats.code.as_ref().expect("code leg").repos, 1);
    let bodies = server.snapshot().code_import_bodies;
    let body: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(
        body["head"],
        serde_json::json!(head),
        "the push carries the refreshed head"
    );
}
