//! `GET /api/v1/completions` (`api::completions`) and `GET /api/v1/commands`
//! (new — the machine-readable route/command inventory, AC-10). `commands`
//! derives its subcommand list from clap introspection (`Cli::command()`),
//! so it cannot drift from the real 28-subcommand surface.

use std::collections::BTreeMap;
use std::time::Instant;

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use clap::CommandFactory;
use serde::Serialize;
use serde_json::json;

use crate::api::{self, Ctx};
use crate::cli::Cli;
use crate::serve::AppState;
use crate::serve::routes::{self, RouteEntry, respond, run_blocking};

/// Real subcommands with no HTTP mapping: `tui` owns a terminal, `serve` IS
/// the server (spec Non-Goal 3).
const CLI_ONLY: &[&str] = &["tui", "serve"];

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/completions",
            command: "completions",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/commands",
            command: "commands",
            mutating: false,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/completions", get(completions))
        .route("/api/v1/commands", get(commands))
}

/// `GET /api/v1/completions?shell=` — a shell completion script as a JSON
/// string (`api::completions`). Conn-free: uses `Ctx::lazy`.
async fn completions(
    State(state): State<AppState>,
    Query(req): Query<api::completions::Request>,
) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::completions::run(&mut ctx, req)
    })
    .await;
    respond("completions", result, started)
}

/// One entry in the `GET /api/v1/commands` inventory.
#[derive(Serialize)]
struct CommandInfo {
    /// Kebab-case clap subcommand name.
    name: String,
    /// `"http"` for every subcommand with an `/api/v1` mapping, else
    /// `"cli-only"` (`tui`, `serve`).
    transport: &'static str,
    /// `"METHOD[|METHOD] /api/v1/<path>"` entries for this command, one per
    /// distinct path. Empty for a real subcommand this step hasn't wired a
    /// route for yet — honest, not silently omitted.
    routes: Vec<String>,
}

/// `GET /api/v1/commands` — every real clap subcommand, `help`/`version`
/// excluded, mapped onto its registered `/api/v1` routes (AC-10). No DB, no
/// blocking pool needed — pure in-memory introspection.
async fn commands(State(_state): State<AppState>) -> Response {
    let started = Instant::now();
    let table = routes::table();
    let root = Cli::command();
    let commands: Vec<CommandInfo> = root
        .get_subcommands()
        .filter(|sub| sub.get_name() != "help")
        .map(|sub| command_info(sub.get_name(), &table))
        .collect();
    respond("commands", Ok(json!({ "commands": commands })), started)
}

/// Build one [`CommandInfo`] for `name`, looking up its routes in `table`
/// unless it is [`CLI_ONLY`].
fn command_info(name: &str, table: &[RouteEntry]) -> CommandInfo {
    if CLI_ONLY.contains(&name) {
        return CommandInfo {
            name: name.to_string(),
            transport: "cli-only",
            routes: Vec::new(),
        };
    }
    CommandInfo {
        name: name.to_string(),
        transport: "http",
        routes: routes_for(table, name),
    }
}

/// Render every `table` entry whose `command` matches `name` as
/// `"METHOD[|METHOD] /api/v1<path>"`, collapsing methods that share a path
/// (e.g. `GET /memories/search` + `POST /memories/search` → one entry).
fn routes_for(table: &[RouteEntry], name: &str) -> Vec<String> {
    let mut by_path: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for entry in table.iter().filter(|e| e.command == name) {
        by_path.entry(entry.path).or_default().push(entry.method);
    }
    by_path
        .into_iter()
        .map(|(path, methods)| format!("{} /api/v1{path}", methods.join("|")))
        .collect()
}
