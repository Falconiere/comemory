#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! Credential + backlog seeding shared by the daemon suites (#257) that
//! drive a real engine hub (`exchange_support::Hub`) from a real
//! `DaemonHome`: writing the `auth.json` a real `comemory auth login`
//! would have left, and seeding local pending writes through the same
//! domain function the CLI's `save` command runs.

use comemory::config::Config;
use comemory::domains::memories::{Kind, save};
use comemory::store::{connection, repository_approval};
use comemory::utilities::context::Ctx;

use crate::daemon_support::DaemonHome;
use crate::exchange_support::{self, Hub};

/// The credential a real `comemory auth login` would have left, naming
/// `hub` through its proxy — written directly since these tests never run
/// the device-code flow.
pub fn write_auth(home: &DaemonHome, hub: &Hub) {
    write_auth_for(
        home,
        &hub.api_url(),
        &hub.token(),
        exchange_support::WORKSPACE,
    );
}

/// The credential a real `comemory auth login` would have left for `api_url`.
pub fn write_auth_for(home: &DaemonHome, api_url: &str, secret: &str, workspace_id: &str) {
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
pub fn seed_local_pending(home: &DaemonHome, hub: &Hub, count: usize, repo: &str) {
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
