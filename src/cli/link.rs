//! `comemory link` — cache repo-label → workspace overrides in `config.toml`.
//!
//! The server allowlist remains authoritative; `[sync.repos]` is a local
//! override cache only.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::load_config;
use crate::config::patch::{patch_config_file, section};
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;
use crate::sync::auth_file::AuthFile;

const EXAMPLES: &str = "\
Examples:
  comemory link --workspace ws_abc --repo my-backend
  comemory link --repo codasignal/foo --workspace ws_org";

/// Arguments to `comemory link`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Platform workspace id to bind repo labels to.
    #[arg(long)]
    pub workspace: Option<String>,
    /// Repo label (basename or `owner/name`) to cache.
    #[arg(long)]
    pub repo: Option<String>,
}

/// Write `[sync.repos]` entries via `config::patch`.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let cfg = load_config(&paths)?;
    let workspace = resolve_workspace(a.workspace.as_deref(), &paths, &cfg)?;
    let repo = a
        .repo
        .ok_or_else(|| Error::Usage("--repo is required (repo label to link)".into()))?;
    patch_config_file(paths.config_file().as_path(), |root| {
        let sync = section(root, "sync")?;
        let repos = section(sync, "repos")?;
        repos.insert(repo.clone(), toml::Value::String(workspace.clone()));
        Ok(())
    })?;
    if json_flag {
        json::write(&serde_json::json!({
            "repo": repo,
            "workspace": workspace,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "Linked `{repo}` → workspace `{workspace}` in config.toml ([sync.repos])"
        )?;
        writeln!(
            out,
            "Note: the server org-repo allowlist still wins at push time."
        )?;
    }
    Ok(())
}

fn resolve_workspace(
    flag: Option<&str>,
    paths: &Paths,
    cfg: &crate::config::Config,
) -> Result<String> {
    if let Some(ws) = flag {
        return Ok(ws.to_string());
    }
    if let Some(ws) = cfg.sync.default_workspace.clone() {
        return Ok(ws);
    }
    if let Some(auth) = AuthFile::load(paths)? {
        return Ok(auth.personal_workspace_id);
    }
    Err(Error::Usage(
        "workspace required: pass --workspace or set sync.default_workspace".into(),
    ))
}
