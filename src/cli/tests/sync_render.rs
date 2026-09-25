#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The code lines `comemory sync` and the login report print.

use comemory::cli::sync_render::{code_line, code_summary_line};
use comemory::domains::sync::code::CodePushStats;
use comemory::domains::sync::initial::InitialSyncStats;

fn stats() -> CodePushStats {
    CodePushStats {
        repos: 2,
        files_pushed: 41,
        files_removed: 3,
        batches: 2,
        unchanged: 1,
        skipped_config: 1,
        blocked_repo: 0,
        skipped_worktree: 0,
        skipped_missing_root: 0,
        failed: 0,
        errors: Vec::new(),
    }
}

#[test]
fn the_code_line_names_every_counter_and_only_mentions_failures_when_there_are_any() {
    let quiet = code_line(&stats());
    assert_eq!(
        quiet,
        "code: 2 repo(s) pushed · 41 files · 3 removed · unchanged=1 · skip_repos=1 · blocked_repo=0"
    );
    let mut withheld = stats();
    withheld.skipped_worktree = 2;
    withheld.skipped_missing_root = 1;
    assert_eq!(
        code_line(&withheld),
        "code: 2 repo(s) pushed · 41 files · 3 removed · unchanged=1 · skip_repos=1 · blocked_repo=0 \
         · worktrees=2 · missing_root=1"
    );
    let mut failing = stats();
    failing.failed = 1;
    failing
        .errors
        .push("acme/app: code manifest: HTTP 503".into());
    assert_eq!(
        code_line(&failing),
        "code: 2 repo(s) pushed · 41 files · 3 removed · unchanged=1 · skip_repos=1 · blocked_repo=0 · failed=1"
    );
}

#[test]
fn the_login_report_says_why_when_the_code_push_could_not_run() {
    let mut initial = InitialSyncStats {
        code: stats(),
        ..InitialSyncStats::default()
    };
    assert_eq!(code_summary_line(&initial), code_line(&stats()));
    initial.code_error = Some("sync_state missing after ensure".into());
    assert_eq!(
        code_summary_line(&initial),
        "code: not pushed (sync_state missing after ensure)"
    );
}

/// `--action status` must not promise a push it will never make: the row for
/// a linked worktree, and the row whose root was deleted, both say why.
/// Real git checkouts and a real store — the rule is about what is on disk.
#[test]
fn the_status_rows_name_the_rows_that_are_never_offered() {
    use crate::test_common::{git_commit, git_repo, git_worktree};

    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    git_repo::init_repo(&main);
    git_commit::commit_files(&main, &[("src/a.ts", "export const a = 1;\n")], "init");
    let live = tmp.path().join("live-worktree");
    git_worktree::add_worktree(&main, &live, "live");
    let gone = tmp.path().join("gone-worktree");
    git_worktree::add_worktree(&main, &gone, "gone");
    let leftover = tmp.path().join("leftover-worktree");
    git_worktree::add_worktree(&main, &leftover, "leftover");

    let paths = comemory::config::Paths::new(tmp.path().join("data"));
    paths.ensure_dirs().unwrap();
    let cfg = comemory::config::Config::defaults();
    let mut conn = comemory::store::connection::open(paths.db_path()).unwrap();
    for (label, root) in [
        ("main", &main),
        ("live-wt", &live),
        ("gone-wt", &gone),
        ("left-wt", &leftover),
    ] {
        let mut ctx = comemory::utilities::context::Ctx::borrowed(&paths, &cfg, &mut conn);
        comemory::domains::code::repo_admin::connect(
            &mut ctx,
            comemory::domains::code::repo_admin::ConnectRequest {
                root: root.to_string_lossy().into_owned(),
                repo: Some(label.to_owned()),
                index_now: false,
            },
        )
        .unwrap();
    }
    std::fs::remove_dir_all(&gone).unwrap();
    std::fs::remove_dir_all(&leftover).unwrap();
    std::fs::create_dir_all(&leftover).unwrap();
    std::fs::write(leftover.join("build.log"), "left behind").unwrap();

    let rows = super::code_status_rows(&conn).unwrap();
    // Every field of every row, not just the new one: a connected-but-never-
    // indexed repo has no files and no head, has never been pushed, and
    // therefore reads as moved — which is exactly the promise `withheld`
    // exists to qualify.
    let all: Vec<(&str, usize, bool, bool, Option<&'static str>)> = rows
        .iter()
        .map(|r| {
            (
                r.repo.as_str(),
                r.files,
                r.head.is_none() && r.pushed_head.is_none() && r.pushed_at.is_none(),
                r.moved_since_push,
                r.withheld,
            )
        })
        .collect();
    assert_eq!(
        all,
        [
            ("gone-wt", 0, true, true, Some("missing_root")),
            ("left-wt", 0, true, true, Some("no_checkout")),
            ("live-wt", 0, true, true, Some("worktree")),
            ("main", 0, true, true, None),
        ]
    );
}

/// A real data dir with one saved memory under `repo`, and an org credential
/// for `(api_url, workspace)`.
fn exchange_home(
    api_url: &str,
    secret: &str,
    repo: &str,
) -> (tempfile::TempDir, comemory::config::Paths) {
    use comemory::domains::memories::{Kind, save};

    let home = tempfile::tempdir().unwrap();
    let paths = comemory::config::Paths::new(home.path().join("data"));
    paths.ensure_dirs().unwrap();
    crate::test_common::auth_fixture::seed_org_auth(&paths, api_url, secret, "ws_render");
    let mut cfg = comemory::config::Config::defaults();
    cfg.sync.after_save = false;
    let mut conn = comemory::store::connection::open(paths.db_path()).unwrap();
    let mut ctx = comemory::utilities::context::Ctx::borrowed(&paths, &cfg, &mut conn);
    let request = save::Request {
        body: format!("a decision recorded under {repo} for the exchange report"),
        title: None,
        kind: Kind::Decision,
        repo: repo.to_string(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    };
    save::run(&mut ctx, request, false, None).unwrap();
    (home, paths)
}

/// Approve `repos` for the key through the store API a policy load uses.
fn approve(paths: &comemory::config::Paths, api_url: &str, repos: &[&str]) {
    use comemory::store::sync_exchange::ExchangeKey;
    use comemory::store::sync_policy_snapshot::{self, PolicySnapshot};

    let conn = comemory::store::connection::open(paths.db_path()).unwrap();
    let snapshot = PolicySnapshot {
        revision: 1,
        fingerprint: "rev-1".into(),
        allowlist: repos.iter().map(|r| (*r).to_string()).collect(),
        mappings: std::collections::BTreeMap::new(),
        loaded_at: "2026-09-24T10:00:00Z".into(),
    };
    sync_policy_snapshot::save(&conn, &ExchangeKey::new(api_url, "ws_render"), &snapshot).unwrap();
}

/// Every hold reason and every network state reaches both the `--json` block
/// and the TTY lines, read from a real store: a held save, a held pull
/// position and a stall, under each network state in turn.
#[test]
fn exchange_status_block_reports_every_state() {
    use comemory::cli::sync_exchange_render::exchange_status_lines;
    use comemory::domains::sync::drain::status;
    use comemory::store::replica_outbox::{self, Scope};
    use comemory::store::replica_outbox_hold::{self, Change, Hold, Target};
    use comemory::store::replica_pull_hold::{self, PullHold};
    use comemory::store::sync_exchange::{self, ExchangeKey, ExchangeRow};

    let api_url = "http://127.0.0.1:9/api";
    let (_home, paths) = exchange_home(api_url, "cmk_render", "acme/private");
    let auth = comemory::domains::sync::AuthFile::load(&paths)
        .unwrap()
        .unwrap();
    let conn = comemory::store::connection::open(paths.db_path()).unwrap();
    let key = ExchangeKey::new(api_url, "ws_render");
    let held = replica_outbox::read(&conn, Scope::All, 1)
        .unwrap()
        .remove(0);
    let change = Change::Hold(Some((Hold::Policy, "acme/private")));
    replica_outbox_hold::update(&conn, Target::Operation(&held.operation_id), change, "t").unwrap();
    let pull_hold = PullHold {
        from_sequence: 5,
        to_sequence: 5,
        reason: "secret".into(),
        entity_kind: Some("memory".into()),
        entity_key: Some("m5".into()),
        repository: None,
        policy_revision: None,
    };
    replica_pull_hold::record(&conn, &key, "epoch-a", &pull_hold, "t").unwrap();

    for network in ["ok", "backoff", "auth_suspended", "protocol_error"] {
        let mut row = ExchangeRow::fresh(&key);
        row.protocol = Some("replica-v1".into());
        row.network_state = network.into();
        row.stall_sequence = Some(9);
        row.stall_reason = Some("incompatible_version".into());
        sync_exchange::save(&conn, &row, "t").unwrap();

        let block = status::status(&conn, &auth).unwrap();
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["network"], network);
        assert_eq!(json["protocol"], "replica-v1");
        assert_eq!(json["caught_up"], false, "a stalled key is never caught up");
        for reason in [
            "policy",
            "secret",
            "skip_repos",
            "workspace",
            "incompatible",
            "order",
            "upgrade",
        ] {
            assert!(
                json["outbox"]["held"][reason].is_number(),
                "outbox.held.{reason}: {json}"
            );
        }
        for reason in [
            "policy",
            "pending_local",
            "server_withheld",
            "secret",
            "id_collision",
        ] {
            assert!(
                json["pull"]["held"][reason].is_number(),
                "pull.held.{reason}: {json}"
            );
        }
        assert_eq!(json["outbox"]["held"]["policy"], 1);
        assert_eq!(json["pull"]["held"]["secret"], 1);
        assert_eq!(json["pull"]["stalled_at"], 9);

        let lines = exchange_status_lines(&block).join("\n");
        assert!(lines.contains(&format!("network={network}")), "{lines}");
        assert!(lines.contains("policy=1"), "{lines}");
        assert!(lines.contains("secret=1"), "{lines}");
        assert!(
            lines.contains("stalled before 9: incompatible_version"),
            "{lines}"
        );
    }
}

/// On a `replica-v1` key the run report's `exchange` leg stands in for the
/// legacy `push`, `pull` and `code` legs: a real save drained by a real
/// manual run against a real in-process engine.
#[test]
fn exchange_run_leg_replaces_legacy_legs() {
    use comemory::cli::sync_render::run_json;
    use comemory::domains::sync::drain::session::Legs;
    use comemory::domains::sync::drain::test_support::LiveEngine;
    use comemory::domains::sync::manual;

    let engine = LiveEngine::start();
    let (_home, paths) = exchange_home(&engine.api_url, &engine.token(), "falconiere/comemory");
    approve(&paths, &engine.api_url, &["falconiere/comemory"]);
    let cfg = comemory::config::Config::defaults();
    let mut session = manual::open_session(&paths, &cfg).unwrap();

    let stats = manual::run(&paths, &cfg, &mut session, None, Legs::Both).unwrap();
    let report = run_json("ws_render", &stats);

    for leg in ["push", "pull", "code"] {
        assert!(report[leg].is_null(), "{leg} is replaced: {report}");
    }
    assert_eq!(report["exchange"]["protocol"], "replica-v1", "{report}");
    assert_eq!(report["exchange"]["pushed"], 1, "{report}");
    assert_eq!(report["exchange"]["end"], "caught_up", "{report}");
    assert_eq!(report["exchange"]["network"], "ok", "{report}");
    assert!(
        report["refresh"].is_object(),
        "the refresh still precedes the push"
    );
}
