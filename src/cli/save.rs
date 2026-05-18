//! `qwick save` — write a new memory to disk via `MemoryStore::save`.

use std::io::Read;
use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::{Kind, MemoryStore};
use crate::prelude::*;

/// Arguments to `qwick save`. The positional `body` is optional — if omitted
/// or `-`, the body is read from stdin so callers can pipe content.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Memory body. Use `-` (or omit) to read from stdin.
    pub body: Option<String>,
    /// Memory kind: decision|bug|convention|discovery|pattern|note.
    #[arg(long, default_value = "note")]
    pub kind: String,
    /// Repo name attached to the memory (free-form string).
    #[arg(long, default_value = "")]
    pub repo: String,
    /// Comma-separated tag list (e.g. `database,postgres`).
    #[arg(long, default_value = "")]
    pub tags: String,
    /// Author identifier. Defaults to empty so callers may omit.
    #[arg(long, default_value = "")]
    pub author: String,
    /// Quality rating 1..=5. Defaults to 3.
    #[arg(long, default_value_t = 3)]
    pub quality: u8,
}

/// JSON shape emitted under `--json`.
#[derive(Serialize)]
struct Output {
    id: String,
    path: String,
}

/// Save the body and emit the new memory id + on-disk path.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let body = match a.body.as_deref() {
        Some("-") | None => read_stdin()?,
        Some(s) => s.to_string(),
    };
    let kind = parse_kind(&a.kind)?;
    let tags: Vec<String> = if a.tags.is_empty() {
        Vec::new()
    } else {
        a.tags.split(',').map(|t| t.trim().to_string()).collect()
    };
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let store = MemoryStore::new(paths);
    let rec = store.save(&body, kind, &a.repo, &tags, &a.author, a.quality)?;

    let output = Output {
        id: rec.frontmatter.id.clone(),
        path: rec.path.to_string_lossy().into_owned(),
    };
    let mut out = std::io::stdout().lock();
    if json {
        writeln!(out, "{}", serde_json::to_string(&output)?)?;
    } else {
        writeln!(out, "saved {}", output.id)?;
        writeln!(out, "  path: {}", output.path)?;
    }
    Ok(())
}

fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

fn parse_kind(s: &str) -> Result<Kind> {
    Ok(match s {
        "decision" => Kind::Decision,
        "bug" => Kind::Bug,
        "convention" => Kind::Convention,
        "discovery" => Kind::Discovery,
        "pattern" => Kind::Pattern,
        _ => Kind::Note,
    })
}
