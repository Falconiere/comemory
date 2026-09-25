#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Catalog shape and lookup checks. Command registration parity lives in
//! `tests/mcp__parity.rs`, against the live `Cli::command()` tree.

use std::collections::BTreeSet;

use comemory::mcp::catalog::{self, TOOLS};

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
