//! `comemory rebuild` — drop the SQLite mirror and repopulate it from
//! the on-disk markdown files.
//!
//! Markdown remains the source of truth in v0.2; `comemory.db` is a
//! rebuildable derived cache. When the DB drifts (schema change, corruption,
//! manual deletion), `comemory rebuild` walks every `memories/*.md`, parses
//! the YAML frontmatter, and reinserts the `memories` + `memory_tags` +
//! `memory_fts` rows along with the graph edges harvested from the body.
//!
//! Vectors are intentionally *not* repopulated here: the v0.2 contract is
//! BYO-vector, so re-embedding requires running the caller's embedder
//! against the markdown bodies and piping the result through `comemory save`
//! or a future ingest command. The lexical path (`memory_fts`) is fully
//! restored, which is enough to answer the lexical branch of the router.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::prelude::*;
use crate::store::{connection, memory_row};

/// Arguments to `comemory rebuild`. Currently no flags — the command always
/// drops the entire SQLite mirror and rebuilds from `memories/`. Wrapped in
/// a struct so future opt-in flags (e.g. `--keep-stats`, `--dry-run`) can
/// land without breaking the dispatcher signature.
#[derive(ClapArgs, Debug)]
pub struct Args;

/// Drop `comemory.db` and rebuild every row mirror-able from markdown:
/// `memories`, `memory_tags`, `memory_fts`, and the v0.2 graph edges
/// (`in_repo`, `authored_by`, `tagged`, plus cross-link references parsed
/// from the body).
///
/// `MemoryStore::list()` is the single enumeration surface — it already
/// skips hidden staging files (`.{id}.tmp`), ignores the `.trash/`
/// directory, sorts deterministically, and hands back parsed
/// `MemoryRecord`s with `frontmatter`, `body`, `path`, and `slug` ready to
/// reuse. The actual row write delegates to `store::memory_row::insert` so
/// `save` and `rebuild` cannot drift on the underlying SQL.
pub async fn run(_args: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;

    let db = paths.db_path();
    if db.exists() {
        std::fs::remove_file(&db).map_err(Error::Io)?;
    }
    let mut conn = connection::open(&db)?;
    let tx = conn.transaction()?;

    let store = MemoryStore::new(paths.clone());
    for rec in store.list()? {
        let md_path = rec.path.to_string_lossy();
        memory_row::insert(
            &tx,
            &rec.frontmatter,
            &rec.body,
            rec.slug.as_str(),
            &md_path,
            &rec.frontmatter.tags,
        )?;
    }

    tx.commit()?;
    Ok(())
}
