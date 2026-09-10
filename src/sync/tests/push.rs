#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! Push filtering under organization enforcement (AC-5, AC-6, AC-7).
//!
//! Two filters survive the removal of the per-repo allowlist: an unlabelled
//! memory never leaves the machine, and a label matching `[sync] skip_repos`
//! is withheld. Everything else is offered to the organization — including
//! labels that no allowlist would ever have carried, which is the behaviour
//! change this suite has to pin down.

use comemory::api::{Ctx, save};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::connection;
use comemory::sync::AuthFile;
use comemory::sync::push;

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

/// A store with `memories` already saved, plus an org credential pointing at
/// `api_url`. Returns the paths, connection and config for the push under test.
fn seeded(api_url: &str, secret: &str, mut cfg: Config, memories: &[(&str, &str)]) -> Seeded {
    // `save::run` fires `sync::auto::after_save_best_effort` on a detached
    // thread, which would race this suite's explicit push for the same
    // sync_log cursor and drain it first. These tests exercise `run_push`
    // directly; the auto path has its own coverage at the CLI level.
    cfg.sync.after_save = false;
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    common::auth_fixture::seed_org_auth(
        &paths,
        api_url,
        secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );
    for (body, repo) in memories {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        save::run(&mut ctx, save_req(body, repo), false, None).expect("save");
    }
    Seeded {
        _home: home,
        paths,
        conn,
        cfg,
    }
}

struct Seeded {
    _home: tempfile::TempDir,
    paths: Paths,
    conn: comemory::store::Connection,
    cfg: Config,
}

impl Seeded {
    fn push(&mut self) -> push::PushStats {
        let auth = AuthFile::load(&self.paths).expect("load").expect("auth");
        push::run_push(&self.paths, &self.cfg, &mut self.conn, &auth, None, 100).expect("push")
    }
}

#[test]
fn unlabelled_memory_is_withheld_without_touching_the_network() {
    // Unreachable base: an unlabelled memory must be filtered before any call.
    let mut seeded = seeded(
        "http://127.0.0.1:9",
        "cmk_test",
        Config::defaults(),
        &[("personal scratch note with no repo label", "")],
    );
    let stats = seeded.push();
    assert_eq!(stats.pushed, 0);
    assert_eq!(stats.skipped_personal, 1);
    assert_eq!(stats.skipped_config, 0);
}

#[test]
fn skip_repos_withholds_a_label_the_operator_chose_to_keep_local() {
    let mut cfg = Config::defaults();
    cfg.sync.skip_repos = vec!["acme/secret-*".into()];
    let mut seeded = seeded(
        "http://127.0.0.1:9",
        "cmk_test",
        cfg,
        &[(
            "client work that must not leave this machine",
            "acme/secret-thing",
        )],
    );
    let stats = seeded.push();
    assert_eq!(stats.pushed, 0);
    assert_eq!(stats.skipped_config, 1);
    assert_eq!(stats.skipped_personal, 0);
}

#[test]
fn a_label_no_allowlist_would_have_carried_is_now_pushed() {
    // The behaviour change: before organization enforcement this label was
    // withheld as `skipped_not_in_org`. It is now offered to the org, and the
    // allowlist route must not be consulted to decide that.
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;

    let body = "a memory labelled with a repo no GitHub App allowlist carried";
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

    let mut seeded = seeded(
        &server.base,
        &secret,
        Config::defaults(),
        &[(body, "acme/never-allowlisted")],
    );
    let stats = seeded.push();

    assert_eq!(stats.pushed, 1);
    assert!(stats.last_pushed_seq >= 1);
    assert!(
        !server.saw_path("/v1/sync/status"),
        "org membership is the gate; the allowlist route must not be reached, saw: {:?}",
        server.paths()
    );
    for request in server.requests() {
        assert!(
            request.workspace_header.is_empty(),
            "{} {} carried a workspace header",
            request.method,
            request.path
        );
    }
}

#[test]
fn a_mixed_batch_reports_each_filter_separately() {
    // One push, three fates: withheld as personal, withheld by config, pushed.
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;

    let pushed_body = "the one memory in this batch the organization receives";
    let id = comemory::memory::id::memory_id(pushed_body);
    let content_hash = comemory::memory::id::sha256_hex(pushed_body.trim_end().as_bytes());
    server.update(|st| {
        st.import_results = serde_json::json!([{
            "id": id,
            "content_hash": content_hash,
            "status": "accepted",
            "seq": 1
        }]);
    });

    let mut cfg = Config::defaults();
    cfg.sync.skip_repos = vec!["acme/secret-*".into()];
    let mut seeded = seeded(
        &server.base,
        &secret,
        cfg,
        &[
            ("an unlabelled personal note", ""),
            ("withheld client work", "acme/secret-thing"),
            (pushed_body, "acme/public-thing"),
        ],
    );
    let stats = seeded.push();

    assert_eq!(stats.pushed, 1);
    assert_eq!(stats.skipped_personal, 1);
    assert_eq!(stats.skipped_config, 1);

    let sent = server
        .snapshot()
        .last_import_body
        .expect("an import was sent");
    assert!(sent.contains(&id), "the public memory must be in the body");
    assert!(
        !sent.contains("withheld client work"),
        "a skip_repos match must never appear in an import body: {sent}"
    );
    assert!(
        !sent.contains("unlabelled personal note"),
        "an unlabelled memory must never appear in an import body: {sent}"
    );
}
