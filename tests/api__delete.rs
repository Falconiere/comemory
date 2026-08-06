//! Mirror test for `src/api/delete.rs`. Seeds a real memory via the
//! `comemory` binary, then calls `api::delete::run` directly against a
//! `Ctx` opened on the same data-dir — proving the extracted command core
//! soft-deletes the same way `comemory delete` does (`cli::delete::run` is
//! byte-compat tested against CLI stdout in `tests/cli__delete.rs`; the
//! HTTP surface's AC-5 cross-check lives in
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

#[test]
fn run_soft_deletes_a_live_memory() {
    let home = tempfile::tempdir().expect("tempdir");
    let id = save(&home, "a memory that will be deleted");
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = api::delete::run(&mut ctx, &id).expect("delete run");
    assert_eq!(resp.deleted, id);

    let deleted_at: Option<String> = conn
        .query_row(
            "SELECT deleted_at FROM memories WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .expect("query memories row");
    assert!(deleted_at.is_some(), "deleted_at must be stamped");

    let trash = paths.memories_dir().join(".trash");
    let moved = std::fs::read_dir(&trash)
        .expect("read trash dir")
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().starts_with(&id));
    assert!(moved, "markdown file should have moved into .trash/");
}

#[test]
fn run_errors_on_a_nonexistent_id() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = api::delete::run(&mut ctx, "deadbeef").expect_err("nonexistent id must error");
    assert!(
        matches!(err, comemory::errors::Error::NotFound(_)),
        "unexpected error: {err}"
    );
}
