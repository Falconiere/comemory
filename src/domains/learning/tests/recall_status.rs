#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/learning/recall_status.rs`. Seeds a real
//! save + a real tracked `find` (through `domains::retrieval::find::run`,
//! never a hand-forged `retrieval_log` row) so `recall_status::run` reads
//! genuine store data, then judges it through the real
//! `domains::learning::feedback::run` to prove `pending` empties and
//! `feedback_events` counts the verdict.

use comemory::config::{Config, Paths};
use comemory::domains::learning::feedback;
use comemory::domains::learning::recall_status;
use comemory::domains::memories::Kind;
use comemory::domains::memories::save;
use comemory::errors::Error;
use comemory::retrieval::find;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use tempfile::TempDir;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use time::macros::offset;

const REPO: &str = "app";

fn fresh_paths(dir: &TempDir) -> Paths {
    let paths = Paths::new(dir.path().to_path_buf());
    paths.ensure_dirs().expect("ensure data dirs");
    paths
}

/// Save one real memory under [`REPO`], returning its id.
fn seed_memory(paths: &Paths, conn: &mut rusqlite::Connection, body: &str) -> String {
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(paths, &cfg, conn);
    let resp = save::run(
        &mut ctx,
        save::Request {
            body: body.to_string(),
            title: None,
            kind: Kind::Note,
            repo: REPO.to_string(),
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
    .expect("save memory");
    resp.id
}

/// Run a real tracked memory-domain `find`, writing one `source='find'`
/// `retrieval_log` row scoped to [`REPO`], and return its query id.
fn tracked_find(paths: &Paths, conn: &mut rusqlite::Connection, query: &str) -> String {
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(paths, &cfg, conn);
    let result = find::run(
        &mut ctx,
        find::Request {
            query: query.to_string(),
            k: None,
            offset: 0,
            domain: Some("memory".into()),
            repo: Some(REPO.to_string()),
            kind: None,
            lang: None,
            path: Vec::new(),
            vector: None,
            since: None,
            until: None,
            as_of: None,
        },
        true,
    )
    .expect("tracked find");
    result.query_id.expect("tracked find carries a query_id")
}

fn recall_status_for(
    paths: &Paths,
    conn: &mut rusqlite::Connection,
    repo: Option<&str>,
    since: Option<&str>,
) -> comemory::errors::Result<recall_status::Output> {
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(paths, &cfg, conn);
    recall_status::run(
        &mut ctx,
        recall_status::Request {
            repo: repo.map(str::to_string),
            since: since.map(str::to_string),
        },
    )
}

#[test]
fn a_tracked_recall_is_pending_until_judged() {
    let dir = TempDir::new().expect("tempdir");
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let memory_id = seed_memory(&paths, &mut conn, "recall status covers this memory");
    let query_id = tracked_find(&paths, &mut conn, "recall status");

    let before = recall_status_for(&paths, &mut conn, Some(REPO), None).expect("recall status");
    assert_eq!(before.repo.as_deref(), Some(REPO));
    assert_eq!(before.queries, 1, "one tracked query in the window");
    assert_eq!(before.feedback_events, 0, "no verdict recorded yet");
    assert_eq!(before.saves, 1, "one memory created in the window");
    assert_eq!(before.pending.len(), 1, "the tracked query has no verdict");
    assert_eq!(before.pending[0].query_id, query_id);
    assert!(before.pending[0].returned_ids.contains(&memory_id));

    feedback::run(
        &mut Ctx::borrowed(&paths, &Config::defaults(), &mut conn),
        feedback::Request {
            query_id: query_id.clone(),
            used: vec![memory_id],
            irrelevant: Vec::new(),
            used_code: Vec::new(),
            irrelevant_code: Vec::new(),
            source: None,
        },
    )
    .expect("record feedback");

    let after = recall_status_for(&paths, &mut conn, Some(REPO), None).expect("recall status");
    assert_eq!(after.queries, 1, "the same query still counts as tracked");
    assert_eq!(after.feedback_events, 1, "the verdict now counts");
    assert!(
        after.pending.is_empty(),
        "the query is judged, no longer pending"
    );
}

#[test]
fn a_since_in_the_future_reports_all_zeros_and_no_pending() {
    let dir = TempDir::new().expect("tempdir");
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).expect("open db");
    seed_memory(&paths, &mut conn, "future window body");
    tracked_find(&paths, &mut conn, "future window");

    let report = recall_status_for(&paths, &mut conn, Some(REPO), Some("2099-01-01"))
        .expect("recall status");
    assert_eq!(report.queries, 0);
    assert_eq!(report.feedback_events, 0);
    assert_eq!(report.saves, 0);
    assert!(report.pending.is_empty());
}

#[test]
fn an_unknown_repo_reports_zeros() {
    let dir = TempDir::new().expect("tempdir");
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).expect("open db");
    seed_memory(&paths, &mut conn, "scoped to app only");
    tracked_find(&paths, &mut conn, "scoped to app only");

    let report =
        recall_status_for(&paths, &mut conn, Some("no-such-repo"), None).expect("recall status");
    assert_eq!(report.queries, 0);
    assert_eq!(report.feedback_events, 0);
    assert_eq!(report.saves, 0);
    assert!(report.pending.is_empty());
}

#[test]
fn an_unparsable_since_is_a_usage_error() {
    let dir = TempDir::new().expect("tempdir");
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).expect("open db");

    let err = recall_status_for(&paths, &mut conn, None, Some("not a date"))
        .expect_err("an unparsable since must be refused before the store opens a query");
    assert!(matches!(err, Error::Usage(_)), "got {err:?}");
}

/// An explicit `since` carrying a non-UTC offset names the same instant no
/// matter which offset it is written in. `resolve_since` must normalise it
/// to UTC before the store's plain string `>=` compares it against
/// `memories.created_at` (always UTC `Z` text), so a `since` five hours
/// ahead in wall-clock terms but the same instant as "now" still counts a
/// memory saved at "now" — and the resolved `since` echoes back in `Z`
/// form rather than carrying the `+05:00` offset through.
#[test]
fn an_offset_since_is_normalised_to_utc_before_comparing() {
    let dir = TempDir::new().expect("tempdir");
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).expect("open db");

    let now = OffsetDateTime::now_utc();
    let since_plus5 = now
        .to_offset(offset!(+5:00))
        .format(&Rfc3339)
        .expect("format since at +05:00");

    seed_memory(&paths, &mut conn, "saved right at the since instant");

    let report = recall_status_for(&paths, &mut conn, Some(REPO), Some(&since_plus5))
        .expect("recall status");
    assert_eq!(
        report.saves, 1,
        "the same instant expressed in +05:00 must still count a save made at/after it"
    );
    assert!(
        report.since.ends_with('Z'),
        "since must echo back UTC-normalised, got {}",
        report.since
    );
    assert!(
        !report.since.contains("+05:00"),
        "since must not carry the original offset through, got {}",
        report.since
    );
}

/// A fresh data dir with no `comemory.db` yet must report zeros without
/// creating one, matching `domains::maintenance::stats::run`'s guard.
#[test]
fn a_missing_database_reports_zeros_without_creating_one() {
    let dir = TempDir::new().expect("tempdir");
    let paths = fresh_paths(&dir);
    assert!(!paths.db_path().exists(), "no database yet");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let report = recall_status::run(
        &mut ctx,
        recall_status::Request {
            repo: Some(REPO.to_string()),
            since: None,
        },
    )
    .expect("recall status on a missing database");

    assert_eq!(report.queries, 0);
    assert_eq!(report.feedback_events, 0);
    assert_eq!(report.saves, 0);
    assert!(report.pending.is_empty());
    assert!(
        !paths.db_path().exists(),
        "recall-status must not create comemory.db as a side effect"
    );
}

/// A database that exists but is not SQLite must surface as an error, not
/// a panic and not a silent zero report: the existence guard only covers the
/// missing-file case.
#[test]
fn a_corrupt_database_is_an_error_not_a_zero_report() {
    let dir = TempDir::new().expect("tempdir");
    let paths = fresh_paths(&dir);
    std::fs::create_dir_all(paths.db_path().parent().expect("db parent")).expect("data dir");
    std::fs::write(paths.db_path(), b"this is not a sqlite file\n").expect("write garbage");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let outcome = recall_status::run(
        &mut ctx,
        recall_status::Request {
            repo: Some(REPO.to_string()),
            since: None,
        },
    );
    assert!(
        outcome.is_err(),
        "a corrupt store must propagate an error, got {outcome:?}"
    );
}
