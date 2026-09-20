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
        "code: 2 repo(s) pushed · 41 files · 3 removed · unchanged=1 · skip_repos=1"
    );
    let mut withheld = stats();
    withheld.skipped_worktree = 2;
    withheld.skipped_missing_root = 1;
    assert_eq!(
        code_line(&withheld),
        "code: 2 repo(s) pushed · 41 files · 3 removed · unchanged=1 · skip_repos=1 \
         · worktrees=2 · missing_root=1"
    );
    let mut failing = stats();
    failing.failed = 1;
    failing
        .errors
        .push("acme/app: code manifest: HTTP 503".into());
    assert_eq!(
        code_line(&failing),
        "code: 2 repo(s) pushed · 41 files · 3 removed · unchanged=1 · skip_repos=1 · failed=1"
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
    let withheld: Vec<(&str, Option<&'static str>)> =
        rows.iter().map(|r| (r.repo.as_str(), r.withheld)).collect();
    assert_eq!(
        withheld,
        [
            ("gone-wt", Some("missing_root")),
            ("left-wt", Some("no_checkout")),
            ("live-wt", Some("worktree")),
            ("main", None),
        ]
    );
}
