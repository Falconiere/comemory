#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/mcp/catalog.rs`. The shape checks (eleven unique
//! names, exactly two writers, every description non-empty) plus the parity
//! half this step can already prove: every `command` is a real clap
//! subcommand, walked off the live `Cli::command()` rather than a copied list.

use std::collections::BTreeSet;

use clap::CommandFactory;
use comemory::cli::Cli;
use comemory::mcp::catalog::{self, TOOLS};

/// Every subcommand name the real clap definition exposes.
fn clap_subcommands() -> BTreeSet<String> {
    Cli::command()
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect()
}

#[test]
fn catalog_holds_eleven_uniquely_named_tools() {
    assert_eq!(TOOLS.len(), 11, "catalog size");
    let names: BTreeSet<&str> = TOOLS.iter().map(|t| t.name).collect();
    assert_eq!(names.len(), 11, "duplicate tool name in {names:?}");
}

#[test]
fn exactly_two_tools_mutate() {
    let mutating: Vec<&str> = TOOLS
        .iter()
        .filter(|t| t.mutating)
        .map(|t| t.name)
        .collect();
    assert_eq!(mutating, vec!["save", "feedback"], "mutating tools");
    assert!(catalog::is_mutating("save"));
    assert!(catalog::is_mutating("feedback"));
    assert!(!catalog::is_mutating("find"));
    // An unknown name is not dispatchable at all, so it is not "mutating".
    assert!(!catalog::is_mutating("delete-everything"));
}

#[test]
fn every_command_is_a_real_clap_subcommand() {
    let known = clap_subcommands();
    assert!(
        known.contains("find"),
        "clap walk found no subcommands at all: {known:?}"
    );
    for tool in TOOLS {
        assert!(
            known.contains(tool.command),
            "tool `{}` names command `{}`, which clap does not have",
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
