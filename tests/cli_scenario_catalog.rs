#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Inventory gate for `docs/scenarios/`, the human-readable CLI test plan.
//! It must track the real clap tree and point at tests that exist.
//!
//! Every check walks the built `Cli::command()` (the same inventory
//! `tests/api__parity.rs` and `tests/cli_help_examples.rs` use), so a new
//! subcommand or flag that ships without a scenario entry fails here
//! instead of silently joining a hand-maintained list:
//!
//! 1. every real subcommand has `docs/scenarios/<name>.md`, and every file
//!    there besides `README.md` / `globals.md` names a real subcommand;
//! 2. every non-global long flag and visible alias is spelled `--<long>` in
//!    its command's file; the global flags are spelled in `globals.md`;
//! 3. every backticked `tests/…rs` / `src/…rs` path a scenario cites exists,
//!    and every `path::fn` cites a `fn` that file defines;
//! 4. every flag has a scenario section that names it on its `**Flags:**`
//!    line AND cites a test (`**Covered by:**`) — a flag listed only in the
//!    table is an untested flag;
//! 5. every command's `**HTTP:**` line agrees with the live
//!    `GET /api/v1/commands` inventory of a real `comemory serve`: each
//!    route string appears in the doc, and a `cli-only` command says `none`;
//! 6. every `tests/cli_scenario_*.rs` / `tests/serve_scenario_*.rs` journey
//!    is listed in `README.md`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Command, CommandFactory};
use comemory::cli::Cli;
use regex::Regex;
use serve_bin::ServeHome;

#[path = "common/serve_bin.rs"]
mod serve_bin;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn scenarios_dir() -> PathBuf {
    root().join("docs/scenarios")
}

fn read_scenario(name: &str) -> String {
    let path = scenarios_dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The built top-level command: `build()` propagates global args and adds
/// the auto `help`, so introspection sees exactly what `--help` prints.
fn built_cli() -> Command {
    let mut cmd = Cli::command();
    cmd.build();
    cmd
}

fn real_subcommands(cli: &Command) -> Vec<&Command> {
    cli.get_subcommands()
        .filter(|s| !matches!(s.get_name(), "help" | "version"))
        .collect()
}

/// `--long` spellings a scenario file must contain for one subcommand:
/// every visible, non-global long flag plus its visible aliases.
fn local_long_flags(sub: &Command) -> Vec<String> {
    let mut flags = Vec::new();
    for arg in sub.get_arguments() {
        if arg.is_global_set() || arg.is_hide_set() {
            continue;
        }
        if matches!(arg.get_id().as_str(), "help" | "version") {
            continue;
        }
        if let Some(long) = arg.get_long() {
            flags.push(format!("--{long}"));
        }
        if let Some(aliases) = arg.get_visible_aliases() {
            flags.extend(aliases.into_iter().map(|a| format!("--{a}")));
        }
    }
    flags
}

/// The primary `--long` spelling of every visible, non-global flag (no
/// aliases): the unit the per-flag scenario-coverage check works in.
fn primary_long_flags(sub: &Command) -> Vec<String> {
    sub.get_arguments()
        .filter(|a| !a.is_global_set() && !a.is_hide_set())
        .filter(|a| !matches!(a.get_id().as_str(), "help" | "version"))
        .filter_map(|a| a.get_long().map(|l| format!("--{l}")))
        .collect()
}

fn has_ext(name: &str, ext: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

fn scenario_files() -> BTreeSet<String> {
    fs::read_dir(scenarios_dir())
        .expect("read docs/scenarios")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| has_ext(n, "md"))
        .collect()
}

#[test]
fn every_subcommand_has_a_scenario_file_and_vice_versa() {
    let cli = built_cli();
    let names: BTreeSet<String> = real_subcommands(&cli)
        .iter()
        .map(|s| s.get_name().to_string())
        .collect();
    assert!(!names.is_empty(), "Cli::command() must expose subcommands");

    let files = scenario_files();
    let mut problems = Vec::new();
    for name in &names {
        if !files.contains(&format!("{name}.md")) {
            problems.push(format!(
                "subcommand `{name}` has no docs/scenarios/{name}.md"
            ));
        }
    }
    for file in &files {
        if matches!(file.as_str(), "README.md" | "globals.md") {
            continue;
        }
        let stem = file.trim_end_matches(".md");
        if !names.contains(stem) {
            problems.push(format!("docs/scenarios/{file} names no clap subcommand"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_flag_is_named_in_its_scenario_file() {
    let cli = built_cli();
    let mut problems = Vec::new();

    let globals = read_scenario("globals.md");
    for arg in cli.get_arguments().filter(|a| a.is_global_set()) {
        let long = arg.get_long().expect("global args are long flags");
        if !globals.contains(&format!("--{long}")) {
            problems.push(format!("globals.md does not mention --{long}"));
        }
    }

    for sub in real_subcommands(&cli) {
        let name = sub.get_name();
        let Ok(doc) = fs::read_to_string(scenarios_dir().join(format!("{name}.md"))) else {
            continue; // reported by the inventory test
        };
        for flag in local_long_flags(sub) {
            if !doc.contains(&flag) {
                problems.push(format!("docs/scenarios/{name}.md does not mention {flag}"));
            }
        }
        for arg in sub.get_arguments().filter(|a| a.is_positional()) {
            let value = arg.get_value_names().map_or_else(
                || arg.get_id().as_str().to_uppercase(),
                |names| names[0].to_string(),
            );
            if !doc.contains(&format!("<{value}>")) && !doc.contains(&format!("[{value}]")) {
                problems.push(format!(
                    "docs/scenarios/{name}.md does not mention positional <{value}>"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_cited_test_exists() {
    let cite = Regex::new(r"`((?:tests|src)/[A-Za-z0-9_./-]+\.rs)(?:::([A-Za-z0-9_]+))?`")
        .expect("citation regex");
    let mut problems = Vec::new();
    for file in scenario_files() {
        let doc = read_scenario(&file);
        for cap in cite.captures_iter(&doc) {
            let rel = &cap[1];
            let path = root().join(rel);
            let Ok(source) = fs::read_to_string(&path) else {
                problems.push(format!("docs/scenarios/{file} cites missing file {rel}"));
                continue;
            };
            if let Some(func) = cap.get(2) {
                let needle = format!("fn {}(", func.as_str());
                if !source.contains(&needle) {
                    problems.push(format!(
                        "docs/scenarios/{file} cites {rel}::{} which that file does not define",
                        func.as_str()
                    ));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_flag_has_a_covered_scenario() {
    let cli = built_cli();
    let mut problems = Vec::new();
    for sub in real_subcommands(&cli) {
        let name = sub.get_name();
        let Ok(doc) = fs::read_to_string(scenarios_dir().join(format!("{name}.md"))) else {
            continue; // reported by the inventory test
        };
        let sections: Vec<&str> = doc.split("\n### ").skip(1).collect();
        for flag in primary_long_flags(sub) {
            let covered = sections.iter().any(|sec| {
                let flags_line = sec
                    .lines()
                    .find(|l| l.contains("**Flags:**"))
                    .unwrap_or_default();
                flags_line.contains(&format!("`{flag}`")) && sec.contains("**Covered by:**")
            });
            if !covered {
                problems.push(format!(
                    "docs/scenarios/{name}.md: {flag} has no scenario section naming it \
                     with a **Covered by:** citation"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_command_documents_its_live_http_twin() {
    let srv = ServeHome::new();
    let inventory = srv.get("/commands");
    let commands = inventory["commands"].as_array().expect("commands array");
    assert!(!commands.is_empty(), "GET /commands must list subcommands");
    let mut problems = Vec::new();
    for c in commands {
        let name = c["name"].as_str().expect("command name");
        let Ok(doc) = fs::read_to_string(scenarios_dir().join(format!("{name}.md"))) else {
            continue; // reported by the inventory test
        };
        if !doc.contains("**HTTP:**") {
            problems.push(format!("docs/scenarios/{name}.md has no **HTTP:** line"));
            continue;
        }
        if c["transport"] == "cli-only" {
            let line = doc
                .lines()
                .find(|l| l.starts_with("**HTTP:**"))
                .unwrap_or_default();
            if !line.contains("none") {
                problems.push(format!("docs/scenarios/{name}.md must say HTTP: none"));
            }
            continue;
        }
        for route in c["routes"].as_array().expect("routes array") {
            let route = route.as_str().expect("route string");
            if !doc.contains(&format!("`{route}`")) {
                problems.push(format!("docs/scenarios/{name}.md does not list `{route}`"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_journey_is_indexed_in_readme() {
    let readme = read_scenario("README.md");
    let journeys: Vec<String> = fs::read_dir(root().join("tests"))
        .expect("read tests/")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| {
            (n.starts_with("cli_scenario_") || n.starts_with("serve_scenario_")) && has_ext(n, "rs")
        })
        .filter(|n| n != "cli_scenario_catalog.rs")
        .collect();
    assert!(
        !journeys.is_empty(),
        "no tests/cli_scenario_*.rs journeys found"
    );
    let missing: Vec<&String> = journeys
        .iter()
        .filter(|n| !readme.contains(&format!("tests/{n}")))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/scenarios/README.md journey table is missing: {missing:?}"
    );
}
