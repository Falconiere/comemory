#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The code lines `comemory sync` and the login report print.

use comemory::cli::sync_render::{code_line, code_summary_line};
use comemory::sync::code::CodePushStats;
use comemory::sync::initial::InitialSyncStats;

fn stats() -> CodePushStats {
    CodePushStats {
        repos: 2,
        files_pushed: 41,
        files_removed: 3,
        batches: 2,
        unchanged: 1,
        skipped_config: 1,
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
