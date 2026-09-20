#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/mcp/catalog.rs`. The shape checks (fifteen unique
//! names, exactly three writers, every description non-empty) plus the parity
//! half this step can already prove: every `command` is a real clap
//! subcommand, walked off the live `Cli::command()` rather than a copied list.

use std::collections::BTreeSet;

use clap::{Command as ClapCommand, CommandFactory};
use comemory::cli::Cli;
use comemory::mcp::catalog::{self, TOOLS};

/// Resolve a root or nested clap path such as `architecture scaffold`.
fn command_at_path<'a>(root: &'a ClapCommand, path: &str) -> Option<&'a ClapCommand> {
    path.split_whitespace()
        .try_fold(root, |current, segment| current.find_subcommand(segment))
}

#[test]
fn catalog_holds_fifteen_uniquely_named_tools() {
    assert_eq!(TOOLS.len(), 15, "catalog size");
    let names: BTreeSet<&str> = TOOLS.iter().map(|t| t.name).collect();
    assert_eq!(names.len(), 15, "duplicate tool name in {names:?}");
}

#[test]
fn exactly_three_tools_mutate() {
    let mutating: Vec<&str> = TOOLS
        .iter()
        .filter(|t| t.mutating)
        .map(|t| t.name)
        .collect();
    assert_eq!(
        mutating,
        vec!["save", "architecture_save", "feedback"],
        "mutating tools"
    );
    assert!(catalog::is_mutating("save"));
    assert!(catalog::is_mutating("feedback"));
    assert!(catalog::is_mutating("architecture_save"));
    assert!(!catalog::is_mutating("find"));
    // An unknown name is not dispatchable at all, so it is not "mutating".
    assert!(!catalog::is_mutating("delete-everything"));
}

#[test]
fn every_command_is_a_real_clap_subcommand() {
    let root = Cli::command();
    assert!(
        root.find_subcommand("find").is_some(),
        "clap has no find command"
    );
    for tool in TOOLS {
        assert!(
            command_at_path(&root, tool.command).is_some(),
            "tool `{}` names command path `{}`, which clap does not have",
            tool.name,
            tool.command
        );
    }
}

#[test]
fn every_tool_carries_an_agent_facing_description() {
    for tool in TOOLS {
        assert!(
            tool.description.len() > 60,
            "tool `{}` has a description too thin to budget into a turn: {:?}",
            tool.name,
            tool.description
        );
    }
}

#[test]
fn entry_finds_a_row_by_name_and_only_by_name() {
    let save = catalog::entry("save").expect("save is catalogued");
    assert_eq!(save.command, "save");
    assert!(save.mutating);
    assert!(catalog::entry("recall_status").is_some());
    // The tool name, not the command spelling, is the lookup key.
    assert!(catalog::entry("recall-status").is_none());
    assert!(catalog::entry("").is_none());
}
