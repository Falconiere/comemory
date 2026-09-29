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
fn catalog_holds_eighteen_uniquely_named_tools() {
    assert_eq!(TOOLS.len(), 18, "catalog size");
    let names: BTreeSet<&str> = TOOLS.iter().map(|t| t.name).collect();
    assert_eq!(names.len(), 18, "duplicate tool name in {names:?}");
}

#[test]
fn exactly_four_tools_mutate() {
    let mutating: Vec<&str> = TOOLS
        .iter()
        .filter(|t| t.mutating)
        .map(|t| t.name)
        .collect();
    assert_eq!(
        mutating,
        vec!["save", "architecture_save", "project_evidence", "feedback"],
        vec!["project_propose", "save", "architecture_save", "feedback"],
        "mutating tools"
    );
    assert!(catalog::is_mutating("save"));
    assert!(catalog::is_mutating("feedback"));
    assert!(catalog::is_mutating("architecture_save"));
    assert!(catalog::is_mutating("project_evidence"));
    assert!(catalog::is_mutating("project_propose"));
    assert!(!catalog::is_mutating("find"));
    // An unknown name is not dispatchable at all, so it is not "mutating".
    assert!(!catalog::is_mutating("delete-everything"));
}

/// Human-only verbs get no tool (#315): nothing catalogued, so nothing
/// `tools/list` advertises (`mcp_01_lists_catalog`), names an approval,
/// rejection, change request, completion, cancellation or deletion.
#[test]
fn no_tool_is_named_for_a_human_only_verb() {
    for tool in TOOLS {
        for verb in [
            "approve",
            "reject",
            "request_changes",
            "complete",
            "cancel",
            "delete",
        ] {
            assert!(
                !tool.name.contains(verb),
                "tool `{}` is named for the human verb `{verb}`",
                tool.name
            );
        }
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
