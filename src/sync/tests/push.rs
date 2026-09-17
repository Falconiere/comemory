#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! Push filtering under organization enforcement.
//!
//! One filter survives: a label matching `[sync] skip_repos` is withheld by
//! the operator's own choice. Everything else is offered to the organization —
//! including labels no allowlist would ever have carried, and memories with no
//! label at all, which is the behaviour change this suite pins down.

use comemory::api::save;
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::connection;
use comemory::sync::AuthFile;
use comemory::sync::push;
use comemory::utilities::context::Ctx;

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
    // Keep after_save off even if a future default flips; these tests exercise
    // `run_push` directly.
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
fn an_unlabelled_memory_is_pushed_like_any_other() {
    // The behaviour change (AC-10): `repo` is set from the cwd's git repo at
    // save time, so withholding unlabelled memories made sync eligibility a
    // function of which directory `comemory save` happened to run in.
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;

    let body = "a note saved in a directory that is not a git worktree";
    let id = comemory::memory::id::memory_id(body);
    let content_hash = comemory::utilities::digest::sha256_hex(body.trim_end().as_bytes());
    server.update(|st| {
        st.import_results = serde_json::json!([{
            "id": id,
            "content_hash": content_hash,
            "status": "accepted",
            "seq": 1
        }]);
    });

    let mut seeded = seeded(&server.base, &secret, Config::defaults(), &[(body, "")]);
    let stats = seeded.push();

    assert_eq!(stats.pushed, 1);
    assert_eq!(stats.skipped_config, 0);
    let sent = server
        .snapshot()
        .last_import_body
        .expect("an import was sent");
    assert!(
        sent.contains(&id),
        "the unlabelled memory must be in the import body: {sent}"
    );
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
    let content_hash = comemory::utilities::digest::sha256_hex(body.trim_end().as_bytes());
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
            request.workspace_header.is_none(),
            "{} {} sent a workspace header",
            request.method,
            request.path
        );
    }
}

#[test]
fn a_mixed_batch_reports_each_filter_separately() {
    // One push, two fates now: withheld by config, or offered. The unlabelled
    // memory rides along with the labelled one.
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;

    let pushed_body = "the labelled memory in this batch the organization receives";
    let unlabelled_body = "an unlabelled note that now travels with it";
    let id = comemory::memory::id::memory_id(pushed_body);
    let content_hash = comemory::utilities::digest::sha256_hex(pushed_body.trim_end().as_bytes());
    let unlabelled_id = comemory::memory::id::memory_id(unlabelled_body);
    let unlabelled_hash =
        comemory::utilities::digest::sha256_hex(unlabelled_body.trim_end().as_bytes());
    server.update(|st| {
        st.import_results = serde_json::json!([
            {
                "id": unlabelled_id,
                "content_hash": unlabelled_hash,
                "status": "accepted",
                "seq": 1
            },
            {
                "id": id,
                "content_hash": content_hash,
                "status": "accepted",
                "seq": 2
            }
        ]);
    });

    let mut cfg = Config::defaults();
    cfg.sync.skip_repos = vec!["acme/secret-*".into()];
    let mut seeded = seeded(
        &server.base,
        &secret,
        cfg,
        &[
            (unlabelled_body, ""),
            ("withheld client work", "acme/secret-thing"),
            (pushed_body, "acme/public-thing"),
        ],
    );
    let stats = seeded.push();

    assert_eq!(stats.pushed, 2);
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
        sent.contains(&unlabelled_id),
        "the unlabelled memory must be offered too: {sent}"
    );
}

#[test]
fn repo_not_allowed_does_not_advance_pushed_seq() {
    // The cursor bug: advancing past a gate reject left those seqs never
    // re-offered. An all-`repo_not_allowed` batch must leave `pushed_seq`
    // at its prior value and surface `rejected_repo`.
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;

    let body = "memory the empty org allowlist refuses";
    let id = comemory::memory::id::memory_id(body);
    let content_hash = comemory::utilities::digest::sha256_hex(body.trim_end().as_bytes());
    server.update(|st| {
        st.import_results = serde_json::json!([{
            "id": id,
            "content_hash": content_hash,
            "status": "repo_not_allowed",
            "reason": "repo not in org allowlist"
        }]);
    });

    let mut seeded = seeded(
        &server.base,
        &secret,
        Config::defaults(),
        &[(body, "acme/never-allowlisted")],
    );
    let stats = seeded.push();

    assert_eq!(stats.pushed, 0);
    assert_eq!(stats.rejected_repo, 1);
    assert_eq!(stats.skipped_config, 0);
    assert_eq!(stats.blocked_secrets, 0);
    assert_eq!(stats.last_pushed_seq, 0);
    let sent = server
        .snapshot()
        .last_import_body
        .expect("the gate-rejected memory must still have been offered");
    assert!(
        sent.contains(&id),
        "import body must include the refused id so a retry can re-offer it: {sent}"
    );
    let after =
        comemory::store::sync_state::get(&seeded.conn, common::auth_fixture::FIXTURE_WORKSPACE)
            .expect("state")
            .expect("row")
            .pushed_seq;
    assert_eq!(
        after, 0,
        "a hard-reject batch must not advance pushed_seq, got {after}"
    );
}
