//! `comemory workspaces` — list platform workspaces. CLI-only.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use owo_colors::OwoColorize;

use serde::Serialize;

use crate::cloud;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;
use crate::sync::auth_file::AuthFile;

/// One rendered workspace row (`personal` marks the login's own workspace).
#[derive(Debug, Serialize)]
struct WorkspaceRow {
    id: String,
    name: String,
    personal: bool,
}

const EXAMPLES: &str = "\
Examples:
  comemory workspaces
  comemory workspaces --json";

/// Arguments to `comemory workspaces`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {}

/// List workspaces for the logged-in device key.
pub async fn run(_a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let auth = AuthFile::load(&paths)?
        .ok_or_else(|| Error::Usage("not logged in — run `comemory auth login`".into()))?;
    let secret = auth.effective_secret();
    // 401/403 is also what a captive proxy or an expired session in front of
    // the API returns, so name both causes rather than only the revoked key.
    let listed = cloud::list_workspaces(&auth.api_url, &secret)?.ok_or_else(|| {
        Error::Usage(format!(
            "device key rejected by {} — check network access, then run `comemory auth login` to mint a new key",
            auth.api_url
        ))
    })?;
    let rows: Vec<WorkspaceRow> = listed
        .into_iter()
        .map(|workspace| WorkspaceRow {
            personal: workspace.id == auth.personal_workspace_id,
            id: workspace.id,
            name: workspace.name,
        })
        .collect();
    if json_flag {
        json::write(&serde_json::json!({ "workspaces": rows }))?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    for row in &rows {
        let tag = if row.personal { " (personal)" } else { "" };
        writeln!(out, "{}{tag}", row.name.bold())?;
        writeln!(out, "  id: {}", row.id)?;
    }
    Ok(())
}
