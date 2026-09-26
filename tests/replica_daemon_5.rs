#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The required resident daemon (#257), part 5: backlog and verify
//! robustness against a real engine hub (AC-10) — a large backlog surviving
//! an outage, SIGSTOP/SIGCONT, and `verify_every` timing across a restart.

#[path = "common/daemon_support.rs"]
mod daemon_support;
#[path = "common/exchange_support.rs"]
mod exchange_support;
#[path = "common/fault_proxy.rs"]
mod fault_proxy;
#[path = "common/replica_support.rs"]
mod replica_support;
#[path = "common/sync_platform_server.rs"]
mod sync_platform_server;

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use comemory::config::Config;
use comemory::domains::memories::{Kind, save};
use comemory::domains::sync::daemon::client::{self, Probe};
use comemory::domains::sync::drain::report::End;
use comemory::domains::sync::drain::{
    self,
    session::{Legs, Mode},
};
use comemory::domains::sync::{AuthFile, auth_barrier};
use comemory::store::{connection, repository_approval};
use comemory::utilities::context::Ctx;
use daemon_support::DaemonHome;
use exchange_support::Hub;
use fault_proxy::{Fault, FaultProxy};
use sha2::{Digest as _, Sha256};
use sync_platform_server::{SyncPlatformServer, SyncPlatformState};

const READY: Duration = Duration::from_secs(15);

/// The credential a real `comemory auth login` would have left, naming
/// `hub` through its proxy — written directly since these tests never run
/// the device-code flow.
fn write_auth(home: &DaemonHome, hub: &Hub) {
    write_auth_for(
        home,
        &hub.api_url(),
        &hub.token(),
        exchange_support::WORKSPACE,
    );
}

/// The credential a real `comemory auth login` would have left for `api_url`.
fn write_auth_for(home: &DaemonHome, api_url: &str, secret: &str, workspace_id: &str) {
    let auth = serde_json::json!({
        "version": 2,
        "secret": secret,
        "key_prefix": "cmk_test",
        "api_url": api_url,
        "organization_id": "org_exchange",
        "organization_slug": "exchange",
        "organization_name": "Exchange",
        "workspace_id": workspace_id,
    });
    std::fs::write(
        home.data_dir().join("auth.json"),
        serde_json::to_vec_pretty(&auth).unwrap(),
    )
    .unwrap();
}

/// Save local operations without starting a concurrent inline push.
fn save_pending(paths: &comemory::config::Paths, count: usize, repo: &str) {
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).expect("open client database");
    let mut ctx = Ctx::borrowed(paths, &cfg, &mut conn);
    for n in 0..count {
        let request = save::Request {
            body: format!("bulk seeded pending write {n}"),
            title: None,
            kind: Kind::Note,
            repo: repo.to_string(),
            tags: Vec::new(),
            author: String::new(),
            quality: 3,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        };
        save::run(&mut ctx, request, false, None).expect("seed pending write");
    }
}

/// `count` real local saves via the same domain function the CLI's `save`
/// command runs, `push: false` so every one lands `pending` rather than
/// racing an inline push — 2,100 subprocess spawns would take real minutes,
/// but the in-process core is exactly what a real `save` does short of the
/// process boundary. `repo` is approved both ways a real approval leaves
/// it: the repository-approval map and a policy snapshot naming `hub`'s
/// key — without the snapshot the exchange holds everything under `policy`
/// indefinitely, same as an org whose allowlist was never fetched.
fn seed_local_pending(home: &DaemonHome, hub: &Hub, count: usize, repo: &str) {
    use comemory::store::sync_exchange::ExchangeKey;
    use comemory::store::sync_policy_snapshot::{self, PolicySnapshot};

    let paths = home.paths();
    let conn = connection::open(paths.db_path()).unwrap();
    repository_approval::replace_all(
        &conn,
        &[(repo.to_string(), repo.to_string())],
        "2026-09-26T00:00:00Z",
    )
    .unwrap();
    let key = ExchangeKey::new(&hub.api_url(), exchange_support::WORKSPACE);
    sync_policy_snapshot::save(
        &conn,
        &key,
        &PolicySnapshot {
            revision: 1,
            fingerprint: format!("rev-1-{repo}"),
            allowlist: vec![repo.to_string()],
            mappings: std::collections::BTreeMap::new(),
            loaded_at: "2026-09-26T00:00:00Z".to_string(),
        },
    )
    .unwrap();
    drop(conn);
    save_pending(&paths, count, repo);
}

/// A response held by the real loopback proxy lets the logout barrier land
/// between a completed replica batch and its next boundary, without racing a
/// clock or inspecting the hub database.
#[test]
fn a_cancelled_inline_replica_batch_requests_one_follow_up_without_spinning() {
    let hub = Hub::start();
    let home = DaemonHome::new();
    write_auth(&home, &hub);
    seed_local_pending(&home, &hub, 1, "acme/backend");
    hub.proxy.arm(Fault::HoldRequest {
        path: "/sync/replica/import".into(),
    });

    let paths = home.paths();
    let worker_paths = paths.clone();
    let worker = std::thread::spawn(move || {
        let cfg = Config::defaults();
        let auth = AuthFile::load(&worker_paths)
            .expect("load auth")
            .expect("seeded auth");
        let mut conn = connection::open(worker_paths.db_path()).expect("open client database");
        drain::drain(
            &worker_paths,
            &cfg,
            &mut conn,
            &auth,
            (Mode::Inline(Duration::from_secs(5)), Legs::Push),
        )
        .expect("inline drain")
    });

    assert!(
        hub.proxy.wait_held(Duration::from_secs(5)),
        "the real replica import must be held before cancellation"
    );
    auth_barrier::raise(&paths).expect("raise logout barrier");
    hub.proxy.release();
    let drained = worker.join().expect("join inline drain");

    assert_eq!(drained.exchange.end, End::Cancelled);
    assert!(
        drained.exchange.more,
        "the caller must wake the coordinator for work that may remain after cancellation"
    );
    assert_eq!(
        hub.proxy.requests_to("/sync/replica/import").len(),
        1,
        "cancellation stops at the boundary instead of starting another batch"
    );
}

/// The older platform routes omit `replica-v1`; this holds its real legacy
/// import so cancellation reaches `legacy_pass::iterate` at the next boundary.
#[test]
fn a_cancelled_inline_legacy_batch_requests_one_follow_up_without_spinning() {
    use std::fmt::Write as _;

    let body = "legacy cancellation holds a real import response";
    let id = comemory::domains::memories::id::memory_id(body);
    let mut content_hash = String::with_capacity(64);
    for byte in Sha256::digest(body.as_bytes()) {
        write!(content_hash, "{byte:02x}").unwrap();
    }
    let platform = SyncPlatformServer::start(SyncPlatformState {
        import_results: serde_json::json!([{
            "id": id,
            "content_hash": content_hash,
            "status": "accepted",
            "seq": 1
        }]),
        head_seq: 1,
        ..SyncPlatformState::default()
    });
    let upstream: SocketAddr = platform
        .base
        .strip_prefix("http://")
        .expect("loopback URL")
        .parse()
        .expect("loopback address");
    let proxy = FaultProxy::start(upstream);
    let home = DaemonHome::new();
    write_auth_for(
        &home,
        &proxy.origin(),
        &platform.snapshot().secret,
        "ws-org",
    );
    let paths = home.paths();
    {
        let cfg = Config::defaults();
        let mut conn = connection::open(paths.db_path()).expect("open client database");
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        save::run(
            &mut ctx,
            save::Request {
                body: body.to_string(),
                title: None,
                kind: Kind::Note,
                repo: "falconiere/comemory".to_string(),
                tags: Vec::new(),
                author: String::new(),
                quality: 3,
                supersedes: Vec::new(),
                vector: None,
                ref_file: Vec::new(),
                ref_symbol: Vec::new(),
            },
            false,
            None,
        )
        .expect("seed legacy pending write");
    }
    proxy.arm(Fault::HoldRequest {
        path: "/v1/sync/import".into(),
    });

    let worker_paths = paths.clone();
    let worker = std::thread::spawn(move || {
        let cfg = Config::defaults();
        let auth = AuthFile::load(&worker_paths)
            .expect("load auth")
            .expect("seeded auth");
        let mut conn = connection::open(worker_paths.db_path()).expect("open client database");
        drain::drain(
            &worker_paths,
            &cfg,
            &mut conn,
            &auth,
            (Mode::Inline(Duration::from_secs(5)), Legs::Push),
        )
        .expect("inline legacy drain")
    });

    assert!(
        proxy.wait_held(Duration::from_secs(5)),
        "the real legacy import must be held before cancellation"
    );
    auth_barrier::raise(&paths).expect("raise logout barrier");
    proxy.release();
    let drained = worker.join().expect("join inline legacy drain");

    assert_eq!(drained.exchange.protocol.as_deref(), Some("legacy"));
    assert!(
        drained.legacy.is_some(),
        "the legacy pass reported its legs"
    );
    assert_eq!(drained.exchange.end, End::Cancelled);
    assert!(
        drained.exchange.more,
        "the caller must wake the coordinator for work that may remain after cancellation"
    );
    assert_eq!(
        proxy.requests_to("/v1/sync/import").len(),
        1,
        "cancellation stops at the boundary instead of starting another legacy batch"
    );
}

/// AC-10: a large backlog answers `status` fast during an outage, reports
/// `end: network`, and drains completely — hub feed count matching, local
/// outbox empty — once the hub returns, with no further command.
#[test]
fn a_backlog_of_2100_pending_operations_survives_a_hub_outage_and_drains_once_it_returns() {
    let hub = Hub::start();
    let home = DaemonHome::new();
    std::fs::write(
        home.data_dir().join("config.toml"),
        "[sync]\ndaemon_interval = \"1s\"\n",
    )
    .unwrap();
    write_auth(&home, &hub);
    // Before the outage and before `ensure`, so the coordinator's very
    // first tick is the one that finds the hub down with the backlog
    // already sitting there.
    seed_local_pending(&home, &hub, 2100, "acme/backend");
    hub.proxy.set_upstream(None);

    home.json(&["sync", "daemon", "ensure"]);
    home.wait_ready(READY);

    let start = Instant::now();
    let status = home.json(&["sync", "--action", "status"]);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "status took {elapsed:?} against a 2,100-op backlog: {status}"
    );
    // The legacy `pending`/`pushed_seq` fields never move for a replica-v1
    // key (that protocol tracks its own outbox); the exchange block is the
    // one this run's own state actually lives in.
    assert!(
        status["exchange"]["outbox"]["pending"]
            .as_i64()
            .unwrap_or(0)
            >= 2100,
        "the backlog must still be local: {status}"
    );
    home.wait_for(Duration::from_secs(10), |r| {
        r.sync
            .last_pass
            .as_ref()
            .is_some_and(|p| p.end == Some(End::Network))
    });

    hub.proxy
        .set_upstream(Some(exchange_support::addr_of(hub.engine())));
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let status = home.json(&["sync", "--action", "status"]);
        if status["exchange"]["outbox"]["pending"].as_i64() == Some(0) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the backlog never drained after the hub returned: {status}"
        );
        std::thread::sleep(Duration::from_millis(300));
    }
    // Each save journals two operations (the memory and its activity_event,
    // per the login/logout suite's own precedent), so 2,100 saves reach the
    // hub as 4,200 — the hub's own feed is the ground truth for "exactly
    // once": a duplicate send would double this count again.
    assert_eq!(
        hub.feed().len(),
        4200,
        "every operation must reach the hub exactly once"
    );
}

/// AC-10: SIGSTOP suspends the coordinator process (a probe against it times
/// out rather than hanging forever); SIGCONT resumes it, and its own passes
/// continue afterward.
#[test]
fn sigstop_and_sigcont_pause_and_resume_the_coordinator_cleanly() {
    let home = DaemonHome::new();
    std::fs::write(
        home.data_dir().join("config.toml"),
        "[sync]\ndaemon_interval = \"1s\"\n",
    )
    .unwrap();
    home.json(&["sync", "daemon", "ensure"]);
    let before = home.wait_ready(READY);

    daemon_support::signal(before.pid, "STOP");
    let stopped = client::probe(&home.paths(), Duration::from_secs(2));
    assert!(
        !matches!(stopped, Probe::Healthy(_)),
        "a suspended coordinator must time out, not hang: {stopped:?}"
    );

    daemon_support::signal(before.pid, "CONT");
    let resumed = home.wait_for(Duration::from_secs(10), |r| r.instance == before.instance);
    assert_eq!(resumed.pid, before.pid);
    home.wait_for(Duration::from_secs(10), |r| {
        r.sync.passes > resumed.sync.passes
    });
}

/// AC-10: `verify_every` writes the stamp and reports `last_verify_at`; a
/// coordinator restarted inside that interval reads the same stamp back
/// rather than verifying again early.
#[test]
fn verify_every_writes_the_stamp_and_a_restart_inside_the_interval_does_not_rerun_it() {
    let hub = Hub::start();
    let home = DaemonHome::new();
    drop(connection::open(home.paths().db_path()).unwrap());
    std::fs::write(
        home.data_dir().join("config.toml"),
        "[sync]\ndaemon_interval = \"1s\"\nverify_every = \"2s\"\n",
    )
    .unwrap();
    write_auth(&home, &hub);
    home.json(&["sync", "daemon", "ensure"]);
    let verified = home.wait_for(Duration::from_secs(10), |r| r.sync.last_verify_at.is_some());
    assert!(home.data_dir().join("sync-verify.last").exists());
    let first_at = verified.sync.last_verify_at.clone().unwrap();

    home.json(&["sync", "daemon", "restart"]);
    let restarted = home.wait_ready(READY);
    assert_ne!(
        restarted.instance, verified.instance,
        "restart must yield a genuinely new instance"
    );
    // Still inside the 2s interval the first verify already covered.
    std::thread::sleep(Duration::from_millis(500));
    let status = home.json(&["sync", "daemon", "status"]);
    assert_eq!(
        status["daemon"]["sync"]["last_verify_at"].as_str(),
        Some(first_at.as_str()),
        "must not verify again before the interval elapses: {status}"
    );
}

/// AC-6: while the fault proxy holds one of the coordinator's own upstream
/// requests open, nothing on the client database is left waiting on it — a
/// fresh `busy_timeout = 0` connection gets `BEGIN IMMEDIATE` at once, a WAL
/// checkpoint reports `busy = 0`, and a concurrent CLI `save` still
/// completes quickly.
#[test]
fn no_sqlite_transaction_spans_a_held_upstream_request() {
    let hub = Hub::start();
    let home = DaemonHome::new();
    drop(connection::open(home.paths().db_path()).unwrap());
    std::fs::write(
        home.data_dir().join("config.toml"),
        "[sync]\ndaemon_interval = \"1s\"\n",
    )
    .unwrap();
    write_auth(&home, &hub);
    home.json(&["sync", "daemon", "ensure"]);
    home.wait_ready(READY);

    // The coordinator's own tick (1s) reaches this within the wait below.
    hub.proxy.arm(Fault::HoldRequest {
        path: "/sync/replica/changes".into(),
    });
    std::thread::sleep(Duration::from_millis(1500));

    let conn = rusqlite::Connection::open(home.data_dir().join("comemory.db")).unwrap();
    conn.busy_timeout(Duration::from_millis(0)).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE; ROLLBACK;")
        .expect("BEGIN IMMEDIATE must not block on a transaction the pass left open");
    let busy: i64 = conn
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        busy, 0,
        "no read or write transaction may be open across the held HTTP call"
    );
    drop(conn);

    let start = Instant::now();
    let (code, _, stderr) = home.run(&[
        "save",
        "--repo",
        "acme/backend",
        "written while upstream is held",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "save took {:?} with the exchange stalled",
        start.elapsed()
    );

    hub.proxy.release();
}
