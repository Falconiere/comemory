use assert_cmd::Command;

#[test]
fn binary_runs_and_prints_version() {
    // Clap's `--version` writes the package name and version to stdout and
    // exits 0; we assert on that so the smoke gate stays meaningful now that
    // bare `qwick-memory` prints help+exit-2 via `arg_required_else_help`.
    Command::cargo_bin("qwick-memory")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicates::str::contains("qwick-memory"));
}
