#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! Push filtering under the server repository policy.

use comemory::config::{Config, Paths};
use comemory::domains::memories::Kind;
use comemory::domains::memories::save;
use comemory::domains::sync::AuthFile;
use comemory::domains::sync::drain::{
    self,
    session::{Legs, Mode},
};
use comemory::domains::sync::push;
use comemory::store::connection;
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
        self.push_result().expect("push")
    }

    /// Drain the push direction the way `comemory sync --action push` does;
    /// a drain that ended on the network is the error it recorded.
    fn push_result(&mut self) -> crate::errors::Result<push::PushStats> {
        let auth = AuthFile::load(&self.paths).expect("load").expect("auth");
        let drained = drain::drain(
            &self.paths,
            &self.cfg,
            &mut self.conn,
            &auth,
            (Mode::Manual, Legs::Push),
        )?;
        if let Some(error) = drained.error {
            return Err(crate::errors::Error::Unavailable(error));
        }
        Ok(drained
            .legacy
            .and_then(|l| l.push)
            .expect("a legacy push leg"))
    }
}

#[test]
fn an_unlabelled_memory_is_withheld() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;

    let body = "a note saved in a directory that is not a git worktree";
    let mut seeded = seeded(&server.base, &secret, Config::defaults(), &[(body, "")]);
    let stats = seeded.push();

    assert_eq!(stats.pushed, 0);
    assert_eq!(stats.blocked_repo, 1);
    assert_eq!(stats.skipped_config, 0);
    assert!(server.snapshot().last_import_body.is_none());
}

#[test]
fn skip_repos_withholds_a_label_the_operator_chose_to_keep_local() {
    let mut cfg = Config::defaults();
    cfg.sync.skip_repos = vec!["acme/secret-*".into()];
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;
    let mut seeded = seeded(
        &server.base,
        &secret,
        cfg,
        &[(
            "client work that must not leave this machine",
            "acme/secret-thing",
        )],
    );
    let stats = seeded.push();
    assert_eq!(stats.pushed, 0);
    assert_eq!(stats.skipped_config, 1);
    assert!(server.snapshot().last_import_body.is_none());
}

#[test]
fn an_admin_mapping_authorizes_a_legacy_label() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;

    let body = "a memory carrying an administrator-confirmed legacy label";
    let id = comemory::domains::memories::id::memory_id(body);
    let content_hash = comemory::utilities::digest::sha256_hex(body.trim_end().as_bytes());
    server.update(|st| {
        st.repo_mappings = serde_json::json!([{
            "label": "acme/legacy",
            "fullName": "falconiere/comemory"
        }]);
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
        &[(body, "acme/legacy")],
    );
    let stats = seeded.push();

    assert_eq!(stats.pushed, 1);
    assert!(stats.last_pushed_seq >= 1);
    assert!(
        server.saw_path("/v1/sync/status"),
        "repository policy must be loaded before import"
    );
    let sent: serde_json::Value =
        serde_json::from_str(&server.snapshot().last_import_body.expect("import"))
            .expect("import json");
    assert_eq!(sent["repositories"][id.as_str()], "falconiere/comemory");
}

#[test]
fn a_mixed_batch_reports_each_filter_separately() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;

    let pushed_body = "the labelled memory in this batch the organization receives";
    let unlabelled_body = "an unlabelled note that must remain local";
    let id = comemory::domains::memories::id::memory_id(pushed_body);
    let content_hash = comemory::utilities::digest::sha256_hex(pushed_body.trim_end().as_bytes());
    let unlabelled_id = comemory::domains::memories::id::memory_id(unlabelled_body);
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
            (unlabelled_body, ""),
            ("withheld client work", "acme/secret-thing"),
            (pushed_body, "Falconiere/Comemory"),
        ],
    );
    let stats = seeded.push();

    assert_eq!(stats.pushed, 1);
    assert_eq!(stats.skipped_config, 1);
    assert_eq!(stats.blocked_repo, 1);

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
        !sent.contains(&unlabelled_id),
        "the unlabelled memory must be withheld: {sent}"
    );
}

#[test]
fn repo_not_allowed_does_not_advance_pushed_seq() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;

    let body = "memory the empty org allowlist refuses";
    let id = comemory::domains::memories::id::memory_id(body);
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
        &[(body, "falconiere/comemory")],
    );
    let error = seeded
        .push_result()
        .expect_err("server rejection must stop push");
    assert!(error.to_string().contains("repository policy"), "{error}");
    assert!(error.to_string().contains(&id), "{error}");
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
