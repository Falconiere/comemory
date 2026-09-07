#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! AC-15 / AC-17 push filter against a real allowlist cache (no network —
//! every candidate is skipped so `push_import` is never called).

use time::OffsetDateTime;

use comemory::api::{Ctx, save};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::connection;
use comemory::sync::AuthFile;
use comemory::sync::allowlist_cache::AllowlistCache;
use comemory::sync::match_key::AllowlistRepo;
use comemory::sync::push;

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

fn seed_auth_and_allowlist(paths: &Paths, workspace_id: &str, repos: Vec<AllowlistRepo>) {
    let auth = AuthFile {
        secret: "cmk_test".into(),
        key_prefix: "cmk_test".into(),
        personal_workspace_id: "ws-personal".into(),
        api_url: "http://127.0.0.1:9".into(), // must not be contacted
        device_name: "test".into(),
        email: None,
    };
    auth.save(paths).expect("auth");
    let cache = AllowlistCache {
        etag: Some("fixture".into()),
        fetched_at: OffsetDateTime::now_utc(),
        repos,
        workspace_id: workspace_id.to_string(),
    };
    cache.save(paths).expect("allowlist");
}

#[test]
fn ac15_personal_repo_skipped_without_network() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    let workspace = "ws-org";
    seed_auth_and_allowlist(
        &paths,
        workspace,
        vec![AllowlistRepo {
            full_name: "codasignal/foo".into(),
            name: "foo".into(),
        }],
    );

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        save::run(
            &mut ctx,
            save_req("personal dotfiles stay local-only for sync", "my-dotfiles"),
            false,
            None,
        )
        .expect("save personal");
    }

    let auth = AuthFile::load(&paths).expect("load").expect("auth");
    let stats = push::run_push(&paths, &cfg, &mut conn, &auth, workspace, None, 100).expect("push");
    assert_eq!(stats.pushed, 0);
    assert_eq!(stats.skipped_not_in_org, 1);
    assert_eq!(stats.skipped_personal, 0);
    assert_eq!(stats.skipped_ambiguous, 0);
}

#[test]
fn ac17_ambiguous_basename_skipped_without_network() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    let workspace = "ws-org";
    seed_auth_and_allowlist(
        &paths,
        workspace,
        vec![
            AllowlistRepo {
                full_name: "org/cli".into(),
                name: "cli".into(),
            },
            AllowlistRepo {
                full_name: "other/cli".into(),
                name: "cli".into(),
            },
        ],
    );

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        save::run(
            &mut ctx,
            save_req("ambiguous bare cli label", "cli"),
            false,
            None,
        )
        .expect("save");
    }

    let auth = AuthFile::load(&paths).expect("load").expect("auth");
    let stats = push::run_push(&paths, &cfg, &mut conn, &auth, workspace, None, 100).expect("push");
    assert_eq!(stats.pushed, 0);
    assert_eq!(stats.skipped_ambiguous, 1);
}

#[test]
fn allowlisted_repo_pushes_over_loopback() {
    use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

    let mut state = SyncPlatformState::default();
    state.import_results = serde_json::json!([{
        "id": "will-replace",
        "content_hash": "aa".repeat(32),
        "status": "accepted",
        "seq": 1
    }]);
    let server = SyncPlatformServer::start(state);
    let secret = server.snapshot().secret;

    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    let workspace = "ws-org";

    AuthFile {
        secret: secret.clone(),
        key_prefix: "cmk_bbbb".into(),
        personal_workspace_id: "ws-personal".into(),
        api_url: server.base.clone(),
        device_name: "test".into(),
        email: None,
    }
    .save(&paths)
    .expect("auth");

    // Stale allowlist cache forces a refresh hit against the platform status.
    let cache = AllowlistCache {
        etag: Some("stale".into()),
        fetched_at: OffsetDateTime::now_utc() - time::Duration::hours(48),
        repos: vec![],
        workspace_id: workspace.to_string(),
    };
    cache.save(&paths).expect("allowlist");

    let body = "allowlisted memory is pushed to the platform import route";
    let id = comemory::memory::id::memory_id(body);
    let content_hash = comemory::memory::id::sha256_hex(body.trim_end().as_bytes());
    server.update(|st| {
        st.import_results = serde_json::json!([{
            "id": id,
            "content_hash": content_hash,
            "status": "accepted",
            "seq": 1
        }]);
    });
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        save::run(&mut ctx, save_req(body, "codasignal/foo"), false, None).expect("save");
    }

    let auth = AuthFile::load(&paths).expect("load").expect("auth");
    let stats = push::run_push(&paths, &cfg, &mut conn, &auth, workspace, None, 100).expect("push");
    assert_eq!(stats.pushed, 1);
    assert!(stats.last_pushed_seq >= 1);
}
