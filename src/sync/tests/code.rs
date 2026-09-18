#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The code-index push against the real loopback platform fixture, over a
//! real `index-code` run (AC-11 … AC-16 of the code-graph sync design).

use comemory::config::{Config, Paths};
use comemory::domains::code::index_code::IndexMode;
use comemory::store::connection;
use comemory::sync::AuthFile;
use comemory::sync::code::{run_code_push, run_code_push_if_moved};

use crate::test_common as common;
use crate::test_common::code_sync_fixture as fixture;
use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

struct Rig {
    _home: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: comemory::store::Connection,
    tree: std::path::PathBuf,
    server: SyncPlatformServer,
}

/// A logged-in store with the fixture repo indexed (unless `indexed` is
/// false), pointed at a fresh loopback platform.
fn rig(cfg: Config, indexed: bool) -> Rig {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("data"));
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    common::auth_fixture::seed_org_auth(
        &paths,
        &server.base,
        &secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );
    let tree = fixture::write_ts_repo(home.path());
    if indexed {
        fixture::index(&paths, &cfg, &mut conn, &tree);
    }
    Rig {
        _home: home,
        paths,
        cfg,
        conn,
        tree,
        server,
    }
}

impl Rig {
    fn auth(&self) -> AuthFile {
        AuthFile::load(&self.paths).unwrap().unwrap()
    }

    fn push(&mut self) -> comemory::sync::code::CodePushStats {
        let auth = self.auth();
        run_code_push(&self.cfg, &mut self.conn, &auth).expect("code push")
    }

    fn push_if_moved(&mut self) -> comemory::sync::code::CodePushStats {
        let auth = self.auth();
        run_code_push_if_moved(&self.cfg, &mut self.conn, &auth).expect("code push")
    }

    fn code_paths(&self) -> Vec<String> {
        self.server
            .paths()
            .into_iter()
            .filter(|p| p.starts_with("/v1/sync/code/"))
            .collect()
    }

    fn import_bodies(&self) -> Vec<serde_json::Value> {
        self.server
            .snapshot()
            .code_import_bodies
            .iter()
            .map(|b| serde_json::from_str(b).unwrap())
            .collect()
    }

    fn reindex(&mut self, mode: IndexMode) {
        fixture::index_mode(&self.paths, &self.cfg, &mut self.conn, &self.tree, mode);
    }
}

fn file_paths(body: &serde_json::Value) -> Vec<&str> {
    body["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect()
}

#[test]
fn ac11_no_indexed_repo_means_no_code_request() {
    let mut rig = rig(Config::defaults(), false);
    let stats = rig.push();
    assert_eq!(stats.repos, 0);
    assert!(rig.code_paths().is_empty(), "{:?}", rig.server.paths());
}

#[test]
fn ac11_code_index_off_sends_nothing_even_with_an_index() {
    let mut cfg = Config::defaults();
    cfg.sync.code_index = false;
    let mut rig = rig(cfg, true);
    let stats = rig.push();
    assert_eq!(stats.repos, 0);
    assert!(rig.code_paths().is_empty());
}

#[test]
fn skip_repos_withholds_the_index_too() {
    let mut cfg = Config::defaults();
    cfg.sync.skip_repos = vec!["scr*".into()];
    let mut rig = rig(cfg, true);
    let stats = rig.push();
    assert_eq!(stats.skipped_config, 1);
    assert_eq!(stats.repos, 0);
    assert!(rig.code_paths().is_empty());
}

#[test]
fn ac12_first_push_sends_every_file_and_the_second_only_reads_the_manifest() {
    let mut rig = rig(Config::defaults(), true);
    let first = rig.push();
    assert_eq!(first.repos, 1);
    assert_eq!(first.files_pushed, 3);
    assert_eq!(first.batches, 1);
    assert_eq!(
        rig.code_paths(),
        ["/v1/sync/code/manifest", "/v1/sync/code/import"]
    );
    let bodies = rig.import_bodies();
    assert_eq!(file_paths(&bodies[0]), ["src/a.ts", "src/b.ts", "src/c.ts"]);
    assert!(bodies[0]["head"].is_string(), "head is stamped");
    assert!(
        bodies[0]["cochange"].is_array(),
        "first push carries the co-change set"
    );
    assert!(
        bodies[0]["files"][2]["imports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "src/a.ts"),
        "{}",
        bodies[0]["files"][2]
    );

    let second = rig.push();
    assert_eq!(second.repos, 0);
    assert_eq!(second.unchanged, 1);
    assert_eq!(
        rig.code_paths(),
        [
            "/v1/sync/code/manifest",
            "/v1/sync/code/import",
            "/v1/sync/code/manifest",
        ]
    );
}

#[test]
fn ac13_a_commit_touching_two_files_pushes_exactly_those_two() {
    let mut rig = rig(Config::defaults(), true);
    rig.push();
    common::git_commit::commit_files(
        &rig.tree,
        &[
            (
                "src/a.ts",
                "export function alpha(): number {\n  return 10;\n}\n",
            ),
            (
                "src/b.ts",
                "export function beta(): number {\n  return 20;\n}\n",
            ),
        ],
        "touch a and b",
    );
    rig.reindex(IndexMode::Incremental);
    let stats = rig.push();
    assert_eq!(stats.files_pushed, 2);
    let bodies = rig.import_bodies();
    assert_eq!(file_paths(&bodies[1]), ["src/a.ts", "src/b.ts"]);
    assert!(bodies[1]["removed"].as_array().unwrap().is_empty());
}

#[test]
fn ac14_a_deleted_file_lands_in_removed() {
    let mut rig = rig(Config::defaults(), true);
    rig.push();
    std::fs::remove_file(rig.tree.join("src/c.ts")).unwrap();
    common::git_repo::run_git(&rig.tree, &["add", "-A"]);
    common::git_repo::run_git(&rig.tree, &["commit", "-q", "-m", "drop c"]);
    // A full re-walk is what forgets a file's cursor locally; the
    // incremental walk only ever visits files that still exist.
    rig.reindex(IndexMode::Full);
    let stats = rig.push();
    assert_eq!(stats.files_removed, 1);
    let bodies = rig.import_bodies();
    assert_eq!(bodies[1]["removed"], serde_json::json!(["src/c.ts"]));
    let manifest = rig.server.snapshot().code_manifests[fixture::REPO].clone();
    let held: Vec<&str> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(held, ["src/a.ts", "src/b.ts"]);
}

#[test]
fn ac16_no_request_body_carries_source_text() {
    let mut rig = rig(Config::defaults(), true);
    rig.push();
    let bodies = rig.server.snapshot().code_import_bodies;
    assert!(!bodies.is_empty());
    for body in &bodies {
        assert!(!body.contains("snippet"), "{body}");
        assert!(!body.contains(fixture::SECRET_BODY_LINE), "{body}");
        assert!(body.contains("\"symbol\":\"alpha\""), "{body}");
    }
}

#[test]
fn if_moved_stays_silent_until_the_index_moves() {
    let mut rig = rig(Config::defaults(), true);
    rig.push();
    let quiet = rig.push_if_moved();
    assert_eq!(
        quiet,
        comemory::sync::code::CodePushStats {
            unchanged: 1,
            ..comemory::sync::code::CodePushStats::default()
        }
    );
    assert_eq!(rig.code_paths().len(), 2, "no request without movement");

    common::git_commit::commit_files(
        &rig.tree,
        &[(
            "src/a.ts",
            "export function alpha(): number {\n  return 11;\n}\n",
        )],
        "move a",
    );
    rig.reindex(IndexMode::Incremental);
    let moved = rig.push_if_moved();
    assert_eq!(moved.files_pushed, 1);
    assert_eq!(rig.code_paths().len(), 4);
}

#[test]
fn code_index_off_stays_silent_even_after_the_index_moves() {
    let mut rig = rig(Config::defaults(), true);
    rig.push();
    rig.cfg.sync.code_index = false;
    common::git_commit::commit_files(
        &rig.tree,
        &[(
            "src/a.ts",
            "export function alpha(): number {\n  return 12;\n}\n",
        )],
        "move a with sync off",
    );
    rig.reindex(IndexMode::Incremental);
    let stats = rig.push_if_moved();
    assert_eq!(stats, comemory::sync::code::CodePushStats::default());
    assert_eq!(
        rig.code_paths().len(),
        2,
        "no request once the switch is off"
    );
}

#[test]
fn a_rejected_batch_counts_as_failed_and_is_re_offered() {
    let mut rig = rig(Config::defaults(), true);
    rig.server.update(|st| {
        st.code_import_rejections =
            Some(serde_json::json!([{ "path": "", "reason": "code_quota: full" }]));
    });
    let stats = rig.push();
    assert_eq!(stats.failed, 1);
    assert_eq!(stats.repos, 0);
    assert!(stats.errors[0].contains("code_quota"), "{:?}", stats.errors);

    // Nothing was recorded, so the next push offers the repo again.
    let retry = rig.push();
    assert_eq!(retry.repos, 1);
    assert_eq!(retry.files_pushed, 3);
}
