#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `sync::auto` over real repositories, a real store and — when logged in —
//! the real loopback platform: the pass a git hook or the agent hook fires.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use comemory::config::{Config, Paths};
use comemory::domains::memories::{Kind, save};
use comemory::domains::sync::auto::{AutoOutcome, AutoStats, PASS_LOCK, QUEUE_LOCK, run_auto};
use comemory::store::{connection, repo_marker};
use comemory::utilities::context::Ctx;
use comemory::utilities::file_lock::FileLock;

use crate::test_common as common;
use crate::test_common::code_sync_fixture as fixture;
use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

struct Rig {
    home: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    tree: PathBuf,
}

/// The fixture repo, hooked (inertly) and indexed, in a fresh data dir.
fn rig() -> Rig {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("data"));
    paths.ensure_dirs().unwrap();
    let mut cfg = Config::defaults();
    cfg.sync.push_on_save = false;
    let tree = fixture::write_ts_repo(home.path());
    fixture::install_inert_hooks(&paths, &cfg, &tree);
    let mut conn = connection::open(paths.db_path()).unwrap();
    fixture::index(&paths, &cfg, &mut conn, &tree);
    Rig {
        home,
        paths,
        cfg,
        tree,
    }
}

impl Rig {
    fn last_head(&self) -> Option<String> {
        let conn = connection::open(self.paths.db_path()).unwrap();
        repo_marker::last_head(&conn, fixture::REPO).unwrap()
    }

    fn pass(&self, checkout: Option<&Path>) -> AutoStats {
        match run_auto(&self.paths, &self.cfg, checkout).expect("auto pass") {
            AutoOutcome::Ran(stats) => stats,
            AutoOutcome::Coalesced => panic!("nothing else holds the queue slot"),
        }
    }

    fn touch_b(&self) -> String {
        fixture::commit_hookless(
            &self.tree,
            &[(
                "src/b.ts",
                "export function beta(): number {\n  return 20;\n}\n",
            )],
            "touch b",
        )
    }
}

#[test]
fn a_logged_out_pass_refreshes_a_stale_hooked_repo_without_touching_the_network() {
    let rig = rig();
    let head = rig.touch_b();

    let stats = rig.pass(None);

    assert!(!stats.logged_in);
    let refresh = stats.run.refresh.expect("the refresh always runs");
    assert_eq!(
        (refresh.checked, refresh.refreshed, refresh.failed),
        (1, 1, 0)
    );
    assert!(stats.run.pull.is_none() && stats.run.push.is_none() && stats.run.code.is_none());
    assert_eq!(stats.error, None);
    assert_eq!(rig.last_head().as_deref(), Some(head.as_str()));
}

#[test]
fn a_logged_in_pass_pushes_the_saved_memory_and_then_only_the_changed_file() {
    let rig = rig();
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;
    common::auth_fixture::seed_org_auth(
        &rig.paths,
        &server.base,
        &secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );
    let body = "hooks keep every repo synced without a cd";
    let id = comemory::domains::memories::id::memory_id(body);
    let content_hash = comemory::utilities::digest::sha256_hex(body.as_bytes());
    server.update(|st| {
        st.import_results = serde_json::json!([{
            "id": id, "content_hash": content_hash, "status": "accepted", "seq": 1
        }]);
    });
    let mut conn = connection::open(rig.paths.db_path()).unwrap();
    let mut ctx = Ctx::borrowed(&rig.paths, &rig.cfg, &mut conn);
    save::run(&mut ctx, save_req(body), false, None).expect("save");
    drop(ctx);
    drop(conn);

    let first = rig.pass(None);
    assert!(first.logged_in);
    assert_eq!(first.error, None, "{first:?}");
    assert_eq!(first.run.push.as_ref().expect("push leg").pushed, 1);
    assert_eq!(first.run.code.as_ref().expect("code leg").files_pushed, 3);

    rig.touch_b();
    let second = rig.pass(Some(&rig.tree));

    let refresh = second.run.refresh.as_ref().expect("refresh");
    assert_eq!((refresh.checked, refresh.refreshed), (1, 1), "{refresh:?}");
    let bodies = server.snapshot().code_import_bodies;
    let last: serde_json::Value = serde_json::from_str(bodies.last().unwrap()).unwrap();
    let pushed: Vec<&str> = last["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(pushed, ["src/b.ts"]);
}

#[test]
fn a_trigger_that_finds_a_pass_already_queued_coalesces() {
    let rig = rig();
    let _queued = FileLock::try_acquire(&rig.paths.data_dir().join(QUEUE_LOCK), "test")
        .unwrap()
        .expect("free");

    for checkout in [None, Some(rig.tree.as_path())] {
        let outcome = run_auto(&rig.paths, &rig.cfg, checkout).expect("auto");
        assert!(
            matches!(outcome, AutoOutcome::Coalesced),
            "{checkout:?}: {outcome:?}"
        );
    }
}

#[test]
fn an_unregistered_checkout_waits_its_turn_instead_of_coalescing() {
    let rig = rig();
    let fresh = rig.home.path().join("fresh");
    common::git_repo::init_repo(&fresh);
    common::git_commit::commit_files(&fresh, &[("src/f.ts", "export const f = 1;\n")], "init");
    let queued = FileLock::try_acquire(&rig.paths.data_dir().join(QUEUE_LOCK), "test")
        .unwrap()
        .expect("free");
    let running = FileLock::acquire(&rig.paths.data_dir().join(PASS_LOCK), "test").unwrap();

    let (tx, rx) = mpsc::channel();
    let (paths, cfg, dir) = (rig.paths.clone(), rig.cfg.clone(), fresh.clone());
    let waiter = std::thread::spawn(move || {
        tx.send(run_auto(&paths, &cfg, Some(&dir)).expect("auto"))
            .unwrap();
    });
    assert!(
        rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "it must wait on the running pass, not coalesce into the queued one"
    );
    drop(running);
    drop(queued);
    let outcome = rx.recv_timeout(Duration::from_secs(30)).expect("finishes");
    waiter.join().unwrap();

    let AutoOutcome::Ran(stats) = outcome else {
        panic!("an unregistered checkout runs its own pass");
    };
    assert_eq!(stats.run.refresh.as_ref().unwrap().refreshed, 1);
    let conn = connection::open(rig.paths.db_path()).unwrap();
    assert!(
        repo_marker::all_repos(&conn)
            .unwrap()
            .contains(&"fresh".to_string())
    );
}

#[test]
fn a_path_outside_any_work_tree_is_reported_and_the_pass_still_runs() {
    let rig = rig();
    let head = rig.touch_b();
    let outside = tempfile::tempdir().unwrap();

    let stats = rig.pass(Some(outside.path()));

    let refresh = stats.run.refresh.expect("refresh");
    assert_eq!(refresh.failed, 1);
    assert!(
        refresh.errors[0].ends_with("not inside a git work tree"),
        "{:?}",
        refresh.errors
    );
    assert_eq!(refresh.refreshed, 1, "the sweep still ran");
    assert_eq!(rig.last_head().as_deref(), Some(head.as_str()));
}

fn save_req(body: &str) -> save::Request {
    save::Request {
        body: body.to_string(),
        title: None,
        kind: Kind::Note,
        repo: fixture::CANONICAL_REPO.into(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    }
}
