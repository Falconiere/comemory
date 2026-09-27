#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Contract tests for `comemory erase` and its HTTP twin `POST /api/v1/erase`
//! (#256, B-5): the confirm gate, exactly one entity, `not_found` for an
//! entity never held, and the report — all through the real binary over a
//! real data directory, and a real `comemory serve`.

#[path = "common/cli_bin.rs"]
mod cli_bin;
#[path = "common/serve_bin.rs"]
mod serve_bin;

use cli_bin::CliHome;
use serde_json::{Value, json};
use serve_bin::ServeHome;

/// A word no query uses; finding it after the erase means a copy survived.
const TOKEN: &str = "cliErasedToken5521";

fn body() -> String {
    format!("The erase contract keeps no copy of {TOKEN} once it is gone.")
}

fn save(home: &CliHome) -> String {
    let saved = home.run_json(&["save", &body(), "--kind", "note"]);
    saved["id"].as_str().expect("saved id").to_string()
}

/// `(exit code, stdout, stderr)` of `comemory <args>` over `home`.
fn run(home: &CliHome, args: &[&str]) -> (i32, String, String) {
    let out = home.bin().args(args).output().expect("run comemory");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Every file under `dir` whose bytes contain `needle`.
fn files_holding(dir: &std::path::Path, needle: &str) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return found,
        Err(e) => panic!("cannot list {}: {e}", dir.display()),
    };
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            found.extend(files_holding(&path, needle));
        } else if match std::fs::read(&path) {
            Ok(bytes) => bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
            // A sidecar a live engine removed after the listing holds nothing.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => panic!("cannot read {}: {e}", path.display()),
        } {
            found.push(path);
        }
    }
    found
}

#[test]
fn erase_without_confirm_is_refused_and_erases_nothing() {
    let home = CliHome::new();
    let id = save(&home);

    let (code, _, stderr) = run(&home, &["erase", "--memory", &id]);

    assert_eq!(code, 70, "{stderr}");
    assert!(stderr.contains("confirm"), "{stderr}");
    let shown = home.run_json(&["show", &id]);
    assert_eq!(shown["id"], json!(id), "the memory is untouched");
}

#[test]
fn erase_names_exactly_one_entity() {
    let home = CliHome::new();
    let (neither, _, _) = run(&home, &["erase", "--confirm"]);
    let (both, _, _) = run(
        &home,
        &[
            "erase",
            "--memory",
            "a1b2c3d4",
            "--document",
            "abc",
            "--confirm",
        ],
    );
    assert_eq!(neither, 2, "clap refuses a missing entity");
    assert_eq!(both, 2, "and two at once");
}

#[test]
fn erase_of_a_memory_never_held_is_not_found_and_creates_no_database() {
    let home = CliHome::new();

    let (code, _, stderr) = run(&home, &["erase", "--memory", "0badc0de", "--confirm"]);

    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("not found"), "{stderr}");
    assert!(
        !home.data_dir().join("comemory.db").exists(),
        "an erase of nothing writes nothing"
    );
}

#[test]
fn erase_memory_reports_what_it_removed_and_leaves_no_copy() {
    let home = CliHome::new();
    let id = save(&home);

    let report = home.run_json(&["erase", "--memory", &id, "--confirm"]);

    assert_eq!(report["kind"], json!("memory"));
    assert_eq!(report["key"], json!(id));
    assert_eq!(report["tombstoned"], json!(true));
    assert_eq!(report["payloads_erased"], json!(1));
    assert_eq!(report["wal_truncated"], json!(true));
    assert_eq!(report["snapshots_with_prior_state"], json!([]));
    for field in ["operations_withdrawn", "staged_removed", "replay_blanked"] {
        assert!(report[field].is_u64(), "{field}: {report}");
    }
    let (code, _, _) = run(&home, &["show", &id]);
    assert_eq!(code, 64, "the memory is gone");
    assert_eq!(
        files_holding(&home.data_dir(), TOKEN),
        Vec::<std::path::PathBuf>::new()
    );
}

#[test]
fn erase_prints_a_text_summary_without_json() {
    let home = CliHome::new();
    let id = save(&home);

    let stdout = home.run_ok(&["erase", "--memory", &id, "--confirm"]);

    assert!(
        stdout.starts_with(&format!("erased memory {id}: 1 payloads erased")),
        "{stdout}"
    );
}

#[test]
fn post_erase_is_confirm_gated_answers_not_found_and_erases() {
    let srv = ServeHome::new();
    let saved = srv.post("/memories", &json!({"body": body(), "kind": "note"}));
    let id = saved["id"].as_str().expect("id").to_string();

    let (status, refused) = srv.post_raw("/erase", &json!({"memory": id}));
    assert_eq!(status, 400, "{refused}");
    assert_eq!(refused["error"]["code"], json!("confirmation_required"));

    let (status, missing) = srv.post_raw("/erase", &json!({"memory": "0badc0de", "confirm": true}));
    assert_eq!(status, 404, "{missing}");
    assert_eq!(missing["error"]["code"], json!("not_found"));

    let (status, both) = srv.post_raw(
        "/erase",
        &json!({"memory": id, "document": "abc", "confirm": true}),
    );
    assert_eq!(status, 400, "{both}");

    let report: Value = srv.post("/erase", &json!({"memory": id, "confirm": true}));
    assert_eq!(report["kind"], json!("memory"));
    assert_eq!(report["tombstoned"], json!(true));
    let (status, _) = srv.get_raw(&format!("/memories/{id}"));
    assert_eq!(status, 404, "the server's own connection reads the erase");
}
