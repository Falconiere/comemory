#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! AC-6 parity test — the MCP twin of `tests/api__parity.rs`, and the price
//! of shipping a curated eleven-tool catalog instead of the 94-route table.
//!
//! Three independent checks, all against the real thing:
//!
//! 1. **Command registration** — every `catalog::TOOLS` row names a real clap
//!    subcommand, walked off the live `Cli::command()` tree.
//! 2. **Transport** — a live `comemory serve`'s `GET /api/v1/commands`
//!    reports `mcp` as `transport: "cli-only"` with an EMPTY routes array (a
//!    stdio server must never be startable by an HTTP request), and
//!    `recall-status` as `transport: "http"` with at least one route.
//! 3. **Field mapping** — for every clap arg id of every catalogued
//!    subcommand, `serde_json::from_value::<ParamType>(json!({id: null}))`
//!    must not fail with `unknown field`. `ParamType` is `FeedbackParams` for
//!    `feedback` and the core's own `Request` for the other ten. A deliberate
//!    made-up field is probed too, so a check that had stopped checking fails
//!    loudly instead of passing green.

use clap::{Command as ClapCommand, CommandFactory};
use comemory::cli::Cli;
use comemory::domains::{code, graph, learning, memories};
use comemory::mcp::catalog::{self, TOOLS};
use comemory::mcp::params::{
    ArchitectureSaveParams, ArchitectureShapeParams, ArchitectureShowParams, FeedbackParams,
};
use comemory::retrieval;
use serde_json::json;
use serve_bin::ServeHome;

#[path = "common/serve_bin.rs"]
mod serve_bin;

/// The `(command, arg id)` pairs a catalogued tool does not map: exactly the
/// rows of `tests/api__parity.rs`'s `EXCLUSIONS` that touch this catalog,
/// each for the reason recorded there.
///
/// `--vector-stdin` is the CLI's stdin-body convenience, which collapses into
/// the one `vector` JSON field its `--vector` sibling already owns.
///
/// `search --only` / `search --path` are the interim document-domain path,
/// which lives entirely in `cli::search_only` and never enters
/// `retrieval::search::Request` — so neither adapter maps them, HTTP
/// included. (The design's AC-6 names only the `vector_stdin` rows; these two
/// touch `search`, a catalogued command, so they belong here too.)
///
/// `recall-status`, `show`, `list`, `edges`, `repos` and `feedback` have none.
const EXCLUSIONS: &[(&str, &str)] = &[
    ("save", "vector_stdin"),
    ("find", "vector_stdin"),
    ("search", "vector_stdin"),
    ("search-code", "vector_stdin"),
    ("context", "vector_stdin"),
    ("search", "only"),
    ("search", "path"),
    ("architecture save", "file"),
];

/// Whether deserializing `{ "<arg_id>": null }` as `T` failed specifically
/// because the field is unknown — the drift this test exists to catch.
fn is_unknown_field<T: serde::de::DeserializeOwned>(arg_id: &str) -> bool {
    match serde_json::from_value::<T>(json!({ arg_id: null })) {
        Ok(_) => false,
        Err(e) => e.to_string().contains("unknown field"),
    }
}

/// A [`PROBES`] entry's function pointer type, named so clippy's
/// `type_complexity` has something to point at.
type ProbeFn = fn(&str) -> bool;

/// One probe per catalog tool, keyed by TOOL name (not command name) so the
/// table is indexed the same way `catalog::entry` is.
const PROBES: &[(&str, ProbeFn)] = &[
    ("find", is_unknown_field::<retrieval::find::Request>),
    ("search", is_unknown_field::<retrieval::search::Request>),
    (
        "search_code",
        is_unknown_field::<retrieval::search_code::Request>,
    ),
    ("context", is_unknown_field::<retrieval::context::Request>),
    ("show", is_unknown_field::<memories::show::Request>),
    ("list", is_unknown_field::<memories::list::Request>),
    ("edges", is_unknown_field::<graph::edges::Request>),
    ("repos", is_unknown_field::<code::repos::Request>),
    (
        "recall_status",
        is_unknown_field::<learning::recall_status::Request>,
    ),
    ("save", is_unknown_field::<memories::save::Request>),
    ("feedback", is_unknown_field::<FeedbackParams>),
    (
        "architecture_scaffold",
        is_unknown_field::<ArchitectureShapeParams>,
    ),
    (
        "architecture_save",
        is_unknown_field::<ArchitectureSaveParams>,
    ),
    (
        "architecture_show",
        is_unknown_field::<ArchitectureShowParams>,
    ),
    (
        "architecture_check",
        is_unknown_field::<ArchitectureShapeParams>,
    ),
];

/// Resolve a root or nested clap path such as `architecture scaffold`.
fn command_at_path<'a>(root: &'a ClapCommand, path: &str) -> Option<&'a ClapCommand> {
    path.split_whitespace()
        .try_fold(root, |current, segment| current.find_subcommand(segment))
}

/// This subcommand's arg ids minus clap's auto `help`/`version` and the
/// documented [`EXCLUSIONS`].
fn remaining_arg_ids(sub: &ClapCommand, command: &str) -> Vec<String> {
    sub.get_arguments()
        .map(|a| a.get_id().to_string())
        .filter(|id| id != "help" && id != "version")
        .filter(|id| !EXCLUSIONS.contains(&(command, id.as_str())))
        .collect()
}

#[test]
fn every_catalogued_tool_names_a_real_subcommand() {
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
fn every_clap_arg_maps_to_a_field_on_its_tool_parameter_type() {
    let root = Cli::command();
    let mut problems = Vec::new();
    for tool in TOOLS {
        let Some((_, probe)) = PROBES.iter().find(|(name, _)| *name == tool.name) else {
            panic!(
                "tool `{}` has no PROBES entry — add its parameter type, or the \
                 field check silently skips it",
                tool.name
            );
        };
        let sub = command_at_path(&root, tool.command)
            .unwrap_or_else(|| panic!("clap command path `{}` must resolve", tool.command));
        for id in remaining_arg_ids(sub, tool.command) {
            if probe(&id) {
                problems.push(format!(
                    "tool `{}` (command `{}`) clap arg `{id}` has no matching field on its \
                     MCP parameter type",
                    tool.name, tool.command
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));

    // The check is armed: a field no parameter type has must be reported as
    // unknown by every probe, or the probe above proves nothing.
    for (name, probe) in PROBES {
        assert!(
            probe("definitely_not_a_field_on_any_request"),
            "the probe for `{name}` accepted an invented field — it is checking nothing"
        );
    }
}

#[test]
fn architecture_save_file_exclusion_is_its_positional_model_path() {
    let root = Cli::command();
    let save = command_at_path(&root, "architecture save").expect("architecture save command");
    let file = save
        .get_arguments()
        .find(|argument| argument.get_id() == "file")
        .expect("architecture save file argument");
    assert!(file.is_positional(), "file must remain positional");
    assert_eq!(
        file.get_value_names()
            .map(|names| names.iter().map(ToString::to_string).collect::<Vec<_>>()),
        Some(vec!["FILE".to_string()]),
        "the excluded file argument must remain the model path"
    );
}

#[test]
fn mcp_is_cli_only_and_recall_status_is_http() {
    let srv = ServeHome::new();
    let inventory = srv.get("/commands");
    let commands = inventory["commands"].as_array().expect("commands array");

    let mcp = commands
        .iter()
        .find(|c| c["name"] == json!("mcp"))
        .expect("GET /api/v1/commands is missing subcommand `mcp`");
    assert_eq!(
        mcp["transport"],
        json!("cli-only"),
        "`mcp` IS a server: a stdio server must never be started by an HTTP request"
    );
    assert_eq!(
        mcp["routes"],
        json!([]),
        "`mcp` must carry zero routes — an accidental future HTTP mapping must fail here"
    );

    let recall = commands
        .iter()
        .find(|c| c["name"] == json!("recall-status"))
        .expect("GET /api/v1/commands is missing subcommand `recall-status`");
    assert_eq!(recall["transport"], json!("http"));
    assert!(
        !recall["routes"]
            .as_array()
            .expect("routes array")
            .is_empty(),
        "`recall-status` is a catalogued tool with a real HTTP twin: {recall}"
    );

    let architecture = commands
        .iter()
        .find(|c| c["name"] == json!("architecture"))
        .expect("GET /api/v1/commands is missing architecture");
    assert_eq!(
        architecture["transport"],
        json!("cli-only"),
        "{architecture}"
    );
    assert_eq!(architecture["routes"], json!([]), "{architecture}");

    // Architecture is already read by the console as a tagged memory, so its
    // nested commands deliberately have no HTTP counterpart. Other tools do.
    for tool in TOOLS {
        if tool.command.starts_with("architecture ") {
            continue;
        }
        let entry = commands
            .iter()
            .find(|c| c["name"] == json!(tool.command))
            .unwrap_or_else(|| panic!("GET /commands is missing `{}`", tool.command));
        assert_eq!(
            entry["transport"],
            json!("http"),
            "catalogued command `{}` must have an HTTP twin: {entry}",
            tool.command
        );
    }

    assert!(
        catalog::entry("mcp").is_none(),
        "`mcp` is not a tool: the server does not offer to start another server"
    );
}
