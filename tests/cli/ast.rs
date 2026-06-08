//! `comemory ast` `--lang` gating: only the five compiled-in languages
//! (rust, typescript, javascript, python, go) are accepted. Any other
//! value must exit non-zero with an error message that lists the
//! supported set so callers can self-correct.

use assert_cmd::Command;

#[test]
fn ast_rejects_unsupported_lang() {
    // `--file` is required by clap so we point at a non-existent path; the
    // `--lang` guard must fire before any file IO so the test stays hermetic.
    let bogus_file = std::env::temp_dir().join("comemory-ast-lang-guard.rs");
    let assertion = Command::cargo_bin("comemory")
        .expect("cargo_bin comemory")
        .args(["ast", "pattern", "--lang", "ruby", "--file"])
        .arg(&bogus_file)
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("supported:"),
        "stderr should list supported langs, got: {stderr:?}"
    );
}
