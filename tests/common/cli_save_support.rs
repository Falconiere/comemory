//! Shared helpers for `tests/cli__save.rs` and `tests/cli__save_2.rs`.

use assert_cmd::Command;
use std::fs;

/// Body A for the near-duplicate tests. Measured: simhash Hamming(A, B) = 5
/// (within NEAR_DUP_HAMMING = 8), Hamming(A, C) = 37, Hamming(B, C) = 36.
pub const DUP_BODY_A: &str = "postgres connection pool exhausts under load spikes raise \
     max_connections to fifty and add pgbouncer in transaction mode for the api workers \
     during peak traffic hours";
/// Body B: A with one word changed (`fifty` → `eighty`).
pub const DUP_BODY_B: &str = "postgres connection pool exhausts under load spikes raise \
     max_connections to eighty and add pgbouncer in transaction mode for the api workers \
     during peak traffic hours";
/// Body C: a genuinely distinct topic.
pub const DUP_BODY_C: &str =
    "ast-grep pattern matching finds unwrap calls across the rust codebase quickly";

/// Count `.md` files at the top of `<data_dir>/memories/`, ignoring the
/// `.trash/` subdir and any hidden tmp files. Returns 0 when the directory
/// does not exist yet (the wrong-dim path is allowed to skip `ensure_dirs`
/// in the future without breaking this assertion).
pub fn count_md_files(data_dir: &std::path::Path) -> usize {
    let mem_dir = data_dir.join("memories");
    let Ok(read) = fs::read_dir(&mem_dir) else {
        return 0;
    };
    read.flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.ends_with(".md") && !n.starts_with('.'))
                .unwrap_or(false)
        })
        .count()
}

/// Run `comemory --json save <body> [extra...]` under `home` and parse the
/// JSON output. `extra` is appended after the body so tests can exercise
/// flags like `--supersedes`.
pub fn save_json_args(home: &tempfile::TempDir, body: &str, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["--json", "save", "--kind", "note", body];
    args.extend_from_slice(extra);
    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(&args)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    serde_json::from_str(stdout.trim()).expect("save --json emits one JSON object")
}

/// Run `comemory --json save <body>` under `home` and parse the JSON output.
pub fn save_json(home: &tempfile::TempDir, body: &str) -> serde_json::Value {
    save_json_args(home, body, &[])
}
