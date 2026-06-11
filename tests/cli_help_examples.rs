//! Asserts that every `comemory <subcommand> --help` ends with an
//! `Examples:` block containing at least one `comemory` invocation.

use assert_cmd::Command;

const SUBCOMMANDS: &[&str] = &[
    "save",
    "search",
    "list",
    "delete",
    "feedback",
    "eval",
    "mine",
    "tune",
    "doctor",
    "index-code",
    "ingest-code",
    "ast",
    "context",
    "prune",
    "gc",
    "install-hooks",
    "completions",
];

fn help_for(sub: &str) -> String {
    let out = Command::cargo_bin("comemory")
        .expect("cargo_bin comemory")
        .args([sub, "--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("help text is utf-8")
}

#[test]
fn every_subcommand_help_has_examples_block() {
    let mut missing: Vec<&str> = Vec::new();
    for sub in SUBCOMMANDS {
        let help = help_for(sub);
        let has_block = help.contains("Examples:");
        let has_invocation = help
            .lines()
            .skip_while(|l| !l.contains("Examples:"))
            .any(|l| l.contains("comemory "));
        if !(has_block && has_invocation) {
            missing.push(sub);
        }
    }
    assert!(
        missing.is_empty(),
        "subcommands missing an Examples: block with a comemory invocation: {missing:?}"
    );
}
