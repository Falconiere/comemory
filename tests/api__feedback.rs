#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/feedback.rs`. Calls `api::feedback::run`
//! directly against a `Ctx` opened on a fresh temp data-dir — proving the
//! extracted command core records feedback the same way `comemory feedback`
//! does (`cli::feedback::run` is byte-compat tested against CLI stdout in
//! `tests/cli__feedback.rs`; the HTTP surface's coverage lives in
//! `tests/serve__routes__memories__write.rs`).

use assert_cmd::Command;
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;

fn save(home: &tempfile::TempDir, body: &str) -> String {
    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["--json", "save", body, "--kind", "note", "--repo", "demo"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("save json")["id"]
        .as_str()
        .expect("id")
        .to_string()
}

fn request(query_id: &str, used: Vec<String>) -> api::feedback::Request {
    api::feedback::Request {
        query_id: query_id.to_string(),
        used,
        irrelevant: Vec::new(),
        used_code: Vec::new(),
        irrelevant_code: Vec::new(),
        source: None,
    }
}

/// Every `(memory_id, verdict, provenance)` event row for `query_id`, in
/// insertion order — read back with a second connection, not from the
/// response.
fn events(paths: &Paths, query_id: &str) -> Vec<(String, String, String)> {
    let conn = connection::open(paths.db_path()).expect("open db");
    let mut stmt = conn
        .prepare(
            "SELECT memory_id, verdict, provenance FROM feedback_events \
              WHERE query_id = ?1 ORDER BY id",
        )
        .expect("prepare");
    stmt.query_map([query_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows")
}

#[test]
fn run_records_feedback_for_an_unlogged_query_id() {
    let home = tempfile::tempdir().expect("tempdir");
    let id = save(&home, "a memory used to test feedback recording");
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = api::feedback::run(&mut ctx, request("q-20260610-a1b2c3d4", vec![id]))
        .expect("feedback run");
    assert_eq!(resp.used, 1);
    assert_eq!(resp.irrelevant, 0);
    assert!(!resp.known_query, "query id was never logged");
    assert_eq!(resp.query_id, "q-20260610-a1b2c3d4");
}

#[test]
fn run_rejects_a_malformed_query_id() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = api::feedback::run(&mut ctx, request("not-a-query-id", Vec::new()))
        .expect_err("malformed query id must be rejected");
    assert!(
        err.to_string().contains("invalid query id"),
        "unexpected error: {err}"
    );
}

#[test]
fn run_rejects_an_invalid_memory_id_in_used() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = request("q-20260610-a1b2c3d4", vec!["not-8-hex".to_string()]);
    let err = api::feedback::run(&mut ctx, req).expect_err("invalid memory id must be rejected");
    assert!(
        err.to_string().contains("invalid memory id"),
        "unexpected error: {err}"
    );
}

/// AC-1 + AC-3 (#130): `source: "implicit"` lands on BOTH verdicts of the
/// batch as `provenance = 'implicit'`, the counters bump as for a manual
/// batch, and the response echoes the stored value.
#[test]
fn run_stores_implicit_provenance_for_used_and_irrelevant() {
    let home = tempfile::tempdir().expect("tempdir");
    let used = save(&home, "the connection pool is sized from the worker count");
    let ignored = save(&home, "retry budgets are per tenant, never global");
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let mut req = request("q-20260912-a1b2c3d4", vec![used.clone()]);
    req.irrelevant = vec![ignored.clone()];
    req.source = Some("implicit".into());
    let resp = api::feedback::run(&mut ctx, req).expect("feedback run");
    assert_eq!((resp.used, resp.irrelevant), (1, 1));
    assert_eq!(resp.provenance, "implicit");
    drop(ctx);

    assert_eq!(
        events(&paths, "q-20260912-a1b2c3d4"),
        vec![
            (used.clone(), "used".to_string(), "implicit".to_string()),
            (
                ignored.clone(),
                "irrelevant".to_string(),
                "implicit".to_string()
            ),
        ]
    );
    let (used_count, irrelevant_count): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT used_count FROM feedback WHERE memory_id = ?1), \
                    (SELECT irrelevant_count FROM feedback WHERE memory_id = ?2)",
            [&used, &ignored],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("counters");
    assert_eq!((used_count, irrelevant_count), (1, 1));
}

/// AC-2 (#130): omitted and `"explicit"` both store today's `manual`.
#[test]
fn run_defaults_and_explicit_both_store_manual() {
    let home = tempfile::tempdir().expect("tempdir");
    let id = save(&home, "a memory whose verdicts stay manual");
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let omitted = api::feedback::run(&mut ctx, request("q-20260912-00000001", vec![id.clone()]))
        .expect("omitted source");
    assert_eq!(omitted.provenance, "manual");
    let mut req = request("q-20260912-00000002", vec![id.clone()]);
    req.source = Some("explicit".into());
    let explicit = api::feedback::run(&mut ctx, req).expect("explicit source");
    assert_eq!(explicit.provenance, "manual");
    drop(ctx);

    for query_id in ["q-20260912-00000001", "q-20260912-00000002"] {
        assert_eq!(
            events(&paths, query_id),
            vec![(id.clone(), "used".to_string(), "manual".to_string())],
            "{query_id}"
        );
    }
}

/// AC-6 (#130): a bad `source` is rejected as a `BadRequest` naming the
/// value, BEFORE the database opens — a fresh data dir stays empty, exactly
/// as it does for a malformed query id (the CLI's AC-13 property).
#[test]
fn run_rejects_an_unknown_source_before_opening_the_db() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    for bad in ["manual", "Implicit", ""] {
        let mut req = request("q-20260912-a1b2c3d4", vec!["aaaa0001".into()]);
        req.source = Some(bad.into());
        let err = api::feedback::run(&mut ctx, req).expect_err("bad source must be rejected");
        assert!(
            matches!(err, comemory::errors::Error::BadRequest(_)),
            "{bad:?}: {err:?}"
        );
        assert_eq!(
            err.to_string(),
            format!("bad request: unknown source `{bad}`: expected explicit or implicit")
        );
    }
    assert!(
        !paths.stats_db().exists() && !paths.db_path().exists(),
        "validation must run before the database is created"
    );
}

/// Boundary (#130): `source` with all four lists empty is accepted exactly
/// as the same body without `source` always was — zero rows, zero counts —
/// and `provenance` still names what a verdict WOULD have been stored under.
#[test]
fn run_with_source_and_no_ids_writes_nothing_but_echoes_provenance() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let mut req = request("q-20260912-a1b2c3d4", Vec::new());
    req.source = Some("implicit".into());
    let resp = api::feedback::run(&mut ctx, req).expect("empty batch");
    assert_eq!(
        (
            resp.used,
            resp.irrelevant,
            resp.used_code,
            resp.irrelevant_code
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(resp.provenance, "implicit");
    drop(ctx);
    assert!(events(&paths, "q-20260912-a1b2c3d4").is_empty());
}
