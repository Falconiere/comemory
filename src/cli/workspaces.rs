//! `comemory workspaces` — list platform workspaces. CLI-only.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use owo_colors::OwoColorize;

use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;
use crate::sync::auth_file::AuthFile;
use crate::sync::client;

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
    let mut rows = client::list_workspaces(&auth.api_url, &secret)?;
    for row in &mut rows {
        row.personal = row.id == auth.personal_workspace_id;
    }
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
