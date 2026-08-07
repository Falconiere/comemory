#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/mine.rs`. Seeds a real failed → reworded search
//! pair with used feedback via the real binary, then calls `api::mine::run`
//! directly against a `Ctx` opened on the same data-dir
//! (`cli::mine::run` is byte-compat tested against CLI stdout in
//! `tests/cli__mine.rs`; the HTTP route lives in
//! `tests/serve__routes__maint__admin.rs`).

use assert_cmd::Command;
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

fn run_json(home: &TempDir, args: &[&str]) -> serde_json::Value {
    let mut cmd = bin(home);
    cmd.arg("--json").args(args);
    let assert = cmd.assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("parse JSON envelope")
}

fn json_str(v: &serde_json::Value, field: &str) -> String {
    v.get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("envelope field {field:?} missing in {v}"))
        .to_string()
}

fn expansion_rows(home: &TempDir) -> i64 {
    let db = home.path().join(".comemory").join("comemory.db");
    let conn = connection::open(db).expect("open db");
    conn.query_row("SELECT COUNT(*) FROM query_expansions", [], |r| r.get(0))
        .expect("count query_expansions")
}

fn seed_reformulation_pair(home: &TempDir) {
    let save = run_json(
        home,
        &[
            "save",
            "VecDimMismatch error raised by the dim guard",
            "--kind",
            "bug",
        ],
    );
    let memory_id = json_str(&save, "id");
    run_json(home, &["search", "embedding size error"]);
    let search = run_json(home, &["search", "VecDimMismatch error"]);
    let query_id = json_str(&search, "query_id");
    run_json(home, &["feedback", &query_id, "--used", &memory_id]);
}

#[test]
fn run_report_only_leaves_query_expansions_empty() {
    let home = TempDir::new().expect("tempdir");
    seed_reformulation_pair(&home);

    let paths = Paths::new(home.path().join(".comemory"));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = api::mine::run(&mut ctx, api::mine::Request { apply: false }).expect("mine run");
    assert!(!resp.applied);
    assert_eq!(resp.mappings.len(), 6, "mappings: {:?}", resp.mappings);
    assert_eq!(expansion_rows(&home), 0, "report-only must not write");
}

#[test]
fn run_apply_rebuilds_query_expansions() {
    let home = TempDir::new().expect("tempdir");
    seed_reformulation_pair(&home);

    let paths = Paths::new(home.path().join(".comemory"));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = api::mine::run(&mut ctx, api::mine::Request { apply: true }).expect("mine run");
    assert!(resp.applied);
    assert!(
        resp.mappings
            .iter()
            .any(|m| m.term == "embedding" && m.expansion == "vec" && m.support == 1)
    );
    assert_eq!(expansion_rows(&home), 6, "apply must persist all mappings");
}

#[test]
fn request_rejects_unknown_fields() {
    let err = serde_json::from_value::<api::mine::Request>(serde_json::json!({
        "apply": true,
        "bogus": 1,
    }))
    .expect_err("unknown field must be rejected");
    assert!(err.to_string().contains("unknown field"));
}
