//! `comemory unindex <SOURCE_ID|PATH>` — unregister a document source and
//! delete its derived rows. External files under the source root are
//! never touched (AC-6).

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::edges;
use crate::output::json;
use crate::prelude::*;
use crate::source::registry::Registry;
use crate::store::{connection, document_fts, documents, sources};

const EXAMPLES: &str = "\
Examples:
  # Unregister by source id (see `comemory sources`)
  comemory unindex 3f9c2a1b4e5d6f708192a3b4c5d6e7f8

  # Unregister by the path it was registered under
  comemory unindex ~/notes";

/// Arguments to `comemory unindex`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// The source id (see `comemory sources`) or the path it was
    /// registered under.
    pub target: String,
}

/// JSON/TTY report for one `unindex` run.
#[derive(Serialize, Debug)]
struct Output {
    source_id: String,
    canonical_path: String,
    documents_removed: usize,
}

/// Unregister the source matching `a.target` and delete its derived rows
/// (`source_files`/`documents`/`document_chunks`/`document_fts`) plus the
/// `member_of_source`/`references_document` edges those rows own.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let registry = Registry::new(paths.clone());
    let conn = connection::open(paths.db_path())?;

    let Some(entry) = registry.unregister(&a.target)? else {
        return Err(Error::NotFound(format!(
            "no registered source matches `{}`",
            a.target
        )));
    };

    // document_fts is a virtual table with no FK, so its rows must be
    // deleted explicitly; `sources::delete` below cascades
    // source_files -> documents -> document_chunks via `ON DELETE CASCADE`.
    // `edges` carries no FK to either table, so both edge kinds the link
    // deriver wrote (`member_of_source` keyed by file id, `references_document`
    // keyed by document id) need the same explicit purge.
    let doc_ids = documents::document_ids_for_source(&conn, entry.id.as_str())?;
    for doc_id in &doc_ids {
        document_fts::delete_document(&conn, doc_id)?;
        edges::delete_touching(&conn, "document", doc_id)?;
    }
    for file in sources::list_files_by_source(&conn, entry.id.as_str())? {
        edges::delete_touching(&conn, "file", &file.id)?;
    }
    sources::delete(&conn, entry.id.as_str())?;

    let output = Output {
        source_id: entry.id.to_string(),
        canonical_path: entry.canonical_path.to_string_lossy().into_owned(),
        documents_removed: doc_ids.len(),
    };
    emit(json_flag, &output)
}

/// Emit the unindex report: a JSON object under `--json`, else a two-line
/// TTY summary.
fn emit(json_flag: bool, output: &Output) -> Result<()> {
    if json_flag {
        json::write(output)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "unregistered {} ({})",
        output.source_id, output.canonical_path
    )?;
    writeln!(out, "  documents removed: {}", output.documents_removed)?;
    Ok(())
}
