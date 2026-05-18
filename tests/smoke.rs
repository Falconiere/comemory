use assert_cmd::Command;

#[test]
fn binary_runs_and_prints_version() {
    Command::cargo_bin("qwick")
        .unwrap()
        .assert()
        .success()
        .stdout(predicates::str::contains("qwick"));
}
