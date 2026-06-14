//! Integration tests for `comemory mine`: a real failed → reworded search
//! pair with used feedback drives mining through the real binary, and
//! `--apply` rebuilds `query_expansions` in the on-disk db.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Run a `--json` subcommand to success and parse its stdout envelope.
fn run_json(home: &TempDir, args: &[&str]) -> Value {
    let mut cmd = bin(home);
    cmd.arg("--json").args(args);
    let assert = cmd.assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("parse JSON envelope")
}

/// Extract a required string field from a JSON envelope.
fn json_str(v: &Value, field: &str) -> String {
    v.get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("envelope field {field:?} missing in {v}"))
        .to_string()
}

/// Count rows currently in `query_expansions` in the on-disk db.
fn expansion_rows(home: &TempDir) -> i64 {
    let db = home.path().join(".comemory").join("comemory.db");
    let conn = comemory::store::connection::open(db).expect("open db");
    conn.query_row("SELECT COUNT(*) FROM query_expansions", [], |r| r.get(0))
        .expect("count query_expansions")
}

#[test]
fn mine_reports_and_apply_rebuilds_query_expansions() {
    let home = TempDir::new().expect("tempdir");
    // The memory body carries the fix tokens (vec/dim/mismatch via the
    // identifier split of VecDimMismatch) plus `error`.
    let save = run_json(
        &home,
        &[
            "save",
            "VecDimMismatch error raised by the dim guard",
            "--kind",
            "bug",
        ],
    );
    let memory_id = json_str(&save, "id");

    // Failed query: logged, but never marked used. Tokens {embedding,
    // size, error} share `error` with the rewording below.
    run_json(&home, &["search", "embedding size error"]);
    // Rewording that worked: gets a used-feedback row.
    let search = run_json(&home, &["search", "VecDimMismatch error"]);
    let query_id = json_str(&search, "query_id");
    run_json(&home, &["feedback", &query_id, "--used", &memory_id]);

    // Report-only run: mappings visible, table untouched.
    let report = run_json(&home, &["mine"]);
    assert_eq!(report["applied"].as_bool(), Some(false));
    let mappings = report["mappings"].as_array().expect("mappings array");
    assert_eq!(
        mappings.len(),
        6,
        "failed {{embedding,size}} x fix {{vec,dim,mismatch}}: {report}"
    );
    assert_eq!(expansion_rows(&home), 0, "report-only must not write");

    // --apply rebuilds the table from the mined set.
    let applied = run_json(&home, &["mine", "--apply"]);
    assert_eq!(applied["applied"].as_bool(), Some(true));
    assert!(
        applied["mappings"]
            .as_array()
            .expect("mappings array")
            .iter()
            .any(|m| m["term"] == "embedding" && m["expansion"] == "vec" && m["support"] == 1)
    );
    assert_eq!(expansion_rows(&home), 6, "apply must persist all mappings");
}

#[test]
fn mine_tty_footer_distinguishes_report_from_apply() {
    let home = TempDir::new().expect("tempdir");
    let assert = bin(&home).args(["mine"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        stdout.contains("report only"),
        "report-only footer expected: {stdout:?}"
    );

    let assert = bin(&home).args(["mine", "--apply"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        stdout.contains("(applied)"),
        "applied footer expected: {stdout:?}"
    );
}
