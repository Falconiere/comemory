#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! AC-12 parity test — the single most important test in the whole
//! `serve-http-api-parity` plan (spec §Architecture: "the parity guarantee
//! is bought back explicitly with a clap-introspection test... That test is
//! part of this change, not a follow-up"). A future PR that adds a clap
//! flag without wiring its HTTP `Request` field must fail CI here, loudly.
//!
//! Two independent checks, both walking the real `Cli::command()` tree
//! (`Cli::command().get_subcommands()`, clap's auto `help`/`version`
//! defensively excluded even though the derived, unbuilt `Command` never
//! actually carries them):
//!
//! 1. **Route registration** — every non-`cli-only` subcommand has ≥1
//!    registered `/api/v1` route; `serve` (spec Non-Goal 3: it IS the
//!    server) asserts the *inverse* shape — `transport:"cli-only"` and an
//!    EMPTY `routes` array — so an accidental future HTTP mapping of it
//!    fails just as loudly as a missing one.
//!    This spawns a real `comemory serve` and hits the real
//!    `GET /api/v1/commands` (`src/serve/routes/meta.rs`), which is itself
//!    clap-introspection-driven — end-to-end proof that endpoint is correct
//!    too, not a second hand-rolled route table that could drift from it.
//! 2. **Field mapping** — for every clap arg id on every non-`cli-only`
//!    subcommand, minus the documented exclusions (design.md "§Request
//!    mapping convention"), probe
//!    `serde_json::from_value::<api::<cmd>::Request>(json!({"<id>": null}))`
//!    and require the error is not `"unknown field"` — every `Request`
//!    derives `#[serde(deny_unknown_fields)]` (spec §Architecture), so an
//!    unmapped flag fails the probe loudly, naming the command and arg id.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use assert_cmd::cargo::cargo_bin;
use clap::{Command as ClapCommand, CommandFactory};
use comemory::api;
use comemory::cli::Cli;
use serde_json::json;
use tempfile::TempDir;

/// Real subcommands with no HTTP mapping at all (spec Non-Goal 3): `serve`
/// IS the server. A local, hardcoded mirror of
/// `serve::routes::meta::CLI_ONLY` (private to that module) — deliberate:
/// this test proves the *real* `GET /api/v1/commands` endpoint against an
/// independently-stated expectation, not against whatever that endpoint's
/// own internal constant happens to say today.
const CLI_ONLY: &[&str] = &["serve"];

// ---------------------------------------------------------------------
// Documented per-(command, arg id) exclusions from the HTTP field mapping.
// Every entry is traceable to a sentence in
// docs/toolu/specs/2026-08-06-serve-http-api-parity-design.md
// "§Request mapping convention" (or, for `delete`'s `id`, to the Route map
// entry for that command — see the comment there).
// ---------------------------------------------------------------------
const EXCLUSIONS: &[(&str, &str)] = &[
    // "every `--vector-stdin`" — the stdin-body convenience collapses into
    // one JSON `"vector"` field shared with `--vector`; `--vector-stdin`
    // itself never gets a JSON counterpart. Appears on save/search/
    // search-code/context/find (the subcommands with a `--vector` flag).
    ("save", "vector_stdin"),
    ("find", "vector_stdin"),
    ("search", "vector_stdin"),
    ("search-code", "vector_stdin"),
    ("context", "vector_stdin"),
    // "`index-code --extract` (streams JSONL to stdout)" — DB-free CLI-only
    // path; `api::index_code::Request` (the DB-write path's request type)
    // carries no `extract` field at all (see its module doc).
    ("index-code", "extract"),
    // "`graph --format` (HTTP is always JSON; `dot`/`html` stay CLI)".
    ("graph", "format"),
    // "`search --only` / `search --path` (the interim pre-s9
    // document-domain path lives entirely in `cli::search_only` and never
    // enters `pipeline::search`)". Scoped to `search` only — other
    // subcommands have their own unrelated `path`-named args
    // (`index-code --path`, `index <PATH>`, `ast --file`) that map fine and
    // must NOT be excluded by this entry.
    ("search", "only"),
    ("search", "path"),
    // `comemory delete`'s only flag is the memory id, which the HTTP route
    // map (design.md "### Route map") carries as a URL path segment —
    // `DELETE /api/v1/memories/{id}?confirm=true` — not a JSON body field.
    // `api::delete::run(&mut Ctx, id: &str)` takes a plain `&str`, not a
    // `Request` struct at all, so there is no `Request` type to probe
    // against; excluding `id` here makes that mapping explicit rather than
    // silently skipping `delete` from the field check.
    ("delete", "id"),
];

/// Whether `(command, arg_id)` is a documented exclusion (see
/// [`EXCLUSIONS`]).
fn is_excluded(command: &str, arg_id: &str) -> bool {
    EXCLUSIONS.contains(&(command, arg_id))
}

/// Result of probing one clap arg id against a concrete `api::<cmd>::Request`
/// type via `serde_json::from_value(json!({ "<id>": null }))`.
#[derive(Debug)]
enum ProbeResult {
    /// Deserialization succeeded — `null` happened to satisfy the field
    /// (e.g. an `Option<T>`). Still proves the field is mapped.
    Accepted,
    /// Deserialization failed for a reason other than an unknown field
    /// (wrong type for a required field, missing a *different* required
    /// field, etc.) — still proves this field id IS recognized.
    RejectedOtherReason,
    /// Deserialization failed specifically because `arg_id` is not a field
    /// on the `Request` type — the drift AC-12 exists to catch.
    UnknownField(String),
}

/// The actual probe mechanism (AC-12, verbatim): build `{ "<id>": null }`
/// and try to deserialize it as `T`. `T` is always some `api::<cmd>::Request`
/// (every one `#[derive(Deserialize)] #[serde(deny_unknown_fields)]`).
fn probe<T: serde::de::DeserializeOwned>(arg_id: &str) -> ProbeResult {
    let value = json!({ arg_id: null });
    match serde_json::from_value::<T>(value) {
        Ok(_) => ProbeResult::Accepted,
        Err(e) if e.to_string().contains("unknown field") => {
            ProbeResult::UnknownField(e.to_string())
        }
        Err(_) => ProbeResult::RejectedOtherReason,
    }
}

/// Cuts the ~24 near-identical `fn probe_<cmd>(id) -> ProbeResult { probe::<api::<cmd>::Request>(id) }`
/// bodies down to one line each.
macro_rules! probe_fn {
    ($name:ident, $ty:ty) => {
        fn $name(arg_id: &str) -> ProbeResult {
            probe::<$ty>(arg_id)
        }
    };
}

probe_fn!(probe_ast, api::ast::Request);
probe_fn!(probe_bandit, api::bandit::Request);
probe_fn!(probe_completions, api::completions::Request);
probe_fn!(probe_consolidate, api::consolidate::Request);
probe_fn!(probe_context, api::context::Request);
probe_fn!(probe_doctor, api::doctor::Request);
probe_fn!(probe_edges, api::edges::Request);
probe_fn!(probe_eval, api::eval::Request);
probe_fn!(probe_feedback, api::feedback::Request);
probe_fn!(probe_gc, api::gc::Request);
probe_fn!(probe_graph, api::graph::Request);
probe_fn!(probe_index, api::index::Request);
probe_fn!(probe_index_code, api::index_code::Request);
probe_fn!(probe_install_hooks, api::install_hooks::Request);
probe_fn!(probe_list, api::list::Request);
probe_fn!(probe_mine, api::mine::Request);
probe_fn!(probe_prune, api::prune::Request);
probe_fn!(probe_rebuild, api::rebuild::Request);
probe_fn!(probe_save, api::save::Request);
probe_fn!(probe_search, api::search::Request);
probe_fn!(probe_search_code, api::search_code::Request);
probe_fn!(probe_sources, api::sources::Request);
probe_fn!(probe_find, api::find::Request);
probe_fn!(probe_hooks, api::hooks::Request);
probe_fn!(probe_repos, api::repos::Request);
probe_fn!(probe_show, api::show::Request);
probe_fn!(probe_stats, api::stats::Request);
probe_fn!(probe_tune, api::tune::Request);
probe_fn!(probe_unindex, api::unindex::Request);

/// A [`PROBES`] dispatch entry's function pointer type, factored out of the
/// tuple so clippy's `type_complexity` lint (`-D warnings`) has a named type
/// to point at instead of an inline `fn(&str) -> ProbeResult`.
type ProbeFn = fn(&str) -> ProbeResult;

/// One dispatch entry per real subcommand that owns a concrete
/// `api::<cmd>::Request` type. `delete` (URL path param, no `Request` type)
/// and `ingest-code` (NDJSON body, no clap flags at all — its only "input"
/// is the request body) are deliberately absent: both have an empty
/// post-exclusion arg-id list today, so [`field_check`] never needs to look
/// them up here. If either ever grows a real clap flag, `field_check`'s
/// "missing probe registration" failure below catches it immediately rather
/// than silently skipping the new flag.
const PROBES: &[(&str, ProbeFn)] = &[
    ("ast", probe_ast),
    ("bandit", probe_bandit),
    ("completions", probe_completions),
    ("consolidate", probe_consolidate),
    ("context", probe_context),
    ("doctor", probe_doctor),
    ("edges", probe_edges),
    ("eval", probe_eval),
    ("feedback", probe_feedback),
    ("gc", probe_gc),
    ("graph", probe_graph),
    ("index", probe_index),
    ("index-code", probe_index_code),
    ("install-hooks", probe_install_hooks),
    ("list", probe_list),
    ("mine", probe_mine),
    ("prune", probe_prune),
    ("rebuild", probe_rebuild),
    ("save", probe_save),
    ("search", probe_search),
    ("search-code", probe_search_code),
    ("find", probe_find),
    ("hooks", probe_hooks),
    ("repos", probe_repos),
    ("show", probe_show),
    ("sources", probe_sources),
    ("stats", probe_stats),
    ("tune", probe_tune),
    ("unindex", probe_unindex),
];

/// Every real, non-synthetic clap subcommand name, clap's auto `help`
/// filtered defensively (the derived, unbuilt `Command` this crate hands
/// back never actually carries it, but filtering by id is cheap insurance
/// against a clap version where it does).
fn real_subcommand_names(root: &ClapCommand) -> Vec<String> {
    root.get_subcommands()
        .filter(|s| s.get_name() != "help" && s.get_name() != "version")
        .map(|s| s.get_name().to_string())
        .collect()
}

/// This subcommand's arg ids minus clap's auto `help`/`version` and this
/// command's [`EXCLUSIONS`] entries.
fn remaining_arg_ids(sub: &ClapCommand, command: &str) -> Vec<String> {
    sub.get_arguments()
        .map(|a| a.get_id().to_string())
        .filter(|id| id != "help" && id != "version")
        .filter(|id| !is_excluded(command, id))
        .collect()
}

#[test]
fn every_clap_arg_maps_to_a_request_field_or_a_documented_exclusion() {
    let root = Cli::command();
    for name in real_subcommand_names(&root) {
        if CLI_ONLY.contains(&name.as_str()) {
            continue;
        }
        let sub = root
            .find_subcommand(&name)
            .unwrap_or_else(|| panic!("clap subcommand `{name}` must resolve via find_subcommand"));
        let ids = remaining_arg_ids(sub, &name);
        if ids.is_empty() {
            continue;
        }
        let Some((_, probe_fn)) = PROBES.iter().find(|(cmd, _)| *cmd == name) else {
            panic!(
                "command `{name}` has unprobed arg ids {ids:?} but no entry in PROBES — \
                 add a probe_fn! for its api::<cmd>::Request (or a documented EXCLUSIONS \
                 entry for every id)"
            );
        };
        for id in ids {
            match probe_fn(&id) {
                ProbeResult::Accepted | ProbeResult::RejectedOtherReason => {}
                ProbeResult::UnknownField(msg) => panic!(
                    "command `{name}` clap arg `{id}` has no matching field on its \
                     api::{name}::Request (or an equivalent documented EXCLUSIONS entry) — \
                     deserialize error: {msg}"
                ),
            }
        }
    }
}

/// Kills the spawned server on drop so a panicking assertion cannot leak it.
struct ServerGuard(Child);
impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn `comemory serve` on an ephemeral port, returning the base URL, the
/// session token, and the kill-on-drop guard (mirrors every other
/// `tests/serve__routes__*.rs` file's `spawn_serve`).
fn spawn_serve(home: &TempDir) -> (String, String, ServerGuard) {
    let mut child = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["--json", "serve", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn serve");
    let stdout = child.stdout.take().expect("piped stdout");
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("read banner");
    let guard = ServerGuard(child);
    let info: serde_json::Value = serde_json::from_str(line.trim()).expect("banner is json");
    let port = info["port"].as_u64().expect("port");
    let token = info["token"].as_str().expect("token").to_string();
    (format!("http://127.0.0.1:{port}"), token, guard)
}

/// Assert one `GET /api/v1/commands` entry has the shape [`CLI_ONLY`]
/// membership predicts: cli-only names get `transport:"cli-only"` + empty
/// `routes` (asserted, not skipped — an accidental future HTTP mapping of
/// `serve` must fail this too); every other real subcommand gets
/// `transport:"http"` + a non-empty `routes` array (AC-12).
fn assert_command_entry(name: &str, commands: &[serde_json::Value]) {
    let entry = commands
        .iter()
        .find(|c| c["name"] == json!(name))
        .unwrap_or_else(|| panic!("GET /api/v1/commands is missing subcommand `{name}`"));
    if CLI_ONLY.contains(&name) {
        assert_eq!(
            entry["transport"],
            json!("cli-only"),
            "`{name}` must be transport:\"cli-only\" (spec Non-Goal 3)"
        );
        assert_eq!(
            entry["routes"],
            json!([]),
            "`{name}` must carry zero routes — a future accidental HTTP mapping of \
             a cli-only command must fail this assertion"
        );
        return;
    }
    assert_eq!(
        entry["transport"],
        json!("http"),
        "`{name}` must be transport:\"http\""
    );
    let routes = entry["routes"].as_array().unwrap_or_else(|| {
        panic!(
            "`{name}`.routes must be an array, got {:?}",
            entry["routes"]
        )
    });
    assert!(
        !routes.is_empty(),
        "`{name}` has no registered /api/v1 route — every non-cli-only \
         subcommand must have at least one (AC-12)"
    );
}

#[test]
fn every_non_cli_only_subcommand_has_a_route_and_cli_only_ones_have_none() {
    let root = Cli::command();
    let real_names = real_subcommand_names(&root);
    assert!(
        real_names.len() >= 20,
        "sanity: expected the full real subcommand set, got {real_names:?}"
    );

    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/commands"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("GET /api/v1/commands");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json body");
    assert_eq!(body["ok"], json!(true));
    let commands = body["data"]["commands"]
        .as_array()
        .expect("data.commands is an array");

    for name in &real_names {
        assert_command_entry(name, commands);
    }
}
