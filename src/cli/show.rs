//! `comemory show` — one memory in full: body, frontmatter, activation,
//! references, and code-reference freshness.
//!
//! The lookup itself lives in `api::show` (Binding Rule 1); this module only
//! parses args, resolves the connection, and renders the TTY detail view.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;

/// Example invocations shown at the bottom of `comemory show --help`.
pub const EXAMPLES: &str = "\
Examples:
  # Show one memory in full
  comemory show a1b2c3d4

  # JSON for scripting
  comemory show a1b2c3d4 --json";

/// Arguments to `comemory show`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// 8-hex memory id to show in full.
    pub id: String,
}

/// Show one memory in full. `--json` emits the [`api::show::Response`]
/// verbatim; TTY mode renders a readable detail view. An unknown or
/// soft-deleted id surfaces `Error::NotFound` before anything is printed.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = api::show::run(&mut ctx, api::show::Request { id: a.id })?;

    if json_flag {
        json::write(&resp)
    } else {
        render_tty(&resp)
    }
}

/// Render the TTY detail view: frontmatter fields, the full body, then any
/// direct code references with their freshness verdict.
fn render_tty(resp: &api::show::Response) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "id            {}", resp.id)?;
    writeln!(out, "kind          {}", resp.kind)?;
    writeln!(out, "repo          {}", resp.repo.as_deref().unwrap_or("-"))?;
    writeln!(out, "slug          {}", resp.slug)?;
    writeln!(out, "tags          {}", tags_line(&resp.tags))?;
    writeln!(out, "quality       {}/5", resp.quality)?;
    writeln!(out, "created       {}", resp.created)?;
    writeln!(out, "updated       {}", resp.updated)?;
    writeln!(
        out,
        "accessed      {} time(s), last {}",
        resp.access_count,
        resp.last_accessed.as_deref().unwrap_or("never")
    )?;
    writeln!(out, "activation    {:.3}", resp.activation)?;
    writeln!(out, "rank          {:.3}", resp.rank_score)?;
    if let Some(id) = &resp.superseded_by {
        writeln!(out, "superseded by {id}")?;
    }
    writeln!(out, "path          {}", resp.path)?;
    writeln!(out)?;
    writeln!(out, "{}", resp.title)?;
    writeln!(out)?;
    writeln!(out, "{}", resp.body)?;
    write_code_refs(&mut out, resp)
}

/// `tag1, tag2` (or `-` when there are none), for the `tags` line.
fn tags_line(tags: &[String]) -> String {
    if tags.is_empty() {
        "-".to_string()
    } else {
        tags.join(", ")
    }
}

/// Append the `code refs:` section when `resp` carries any.
fn write_code_refs(out: &mut impl std::io::Write, resp: &api::show::Response) -> Result<()> {
    if resp.code_refs.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(out, "code refs:")?;
    for c in &resp.code_refs {
        writeln!(out, "  {}  [{}]  {}", c.path, c.status, c.anchor)?;
    }
    Ok(())
}
