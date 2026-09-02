#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Asserts that every real `comemory <subcommand> --help` ends with an
//! `Examples:` block containing at least one `comemory` invocation.
//!
//! The inventory is `Cli::command().get_subcommands()`, the same walk
//! `tests/api__parity.rs` uses, so a new subcommand that ships without
//! examples fails this test instead of silently joining a hardcoded list.

use assert_cmd::Command;
use clap::CommandFactory;
use comemory::cli::Cli;

fn real_subcommand_names() -> Vec<String> {
    Cli::command()
        .get_subcommands()
        .filter(|s| s.get_name() != "help" && s.get_name() != "version")
        .map(|s| s.get_name().to_string())
        .collect()
}

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
    let names = real_subcommand_names();
    assert!(
        !names.is_empty(),
        "Cli::command() must expose at least one subcommand"
    );
    let mut missing: Vec<String> = Vec::new();
    for sub in &names {
        let help = help_for(sub);
        let has_block = help.contains("Examples:");
        let has_invocation = help
            .lines()
            .skip_while(|l| !l.contains("Examples:"))
            .any(|l| l.contains("comemory "));
        if !(has_block && has_invocation) {
            missing.push(sub.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "subcommands missing an Examples: block with a comemory invocation: {missing:?}"
    );
}
