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
    }
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
