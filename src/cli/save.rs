//! `comemory save` — atomic markdown write + SQLite-mirror upsert.
//!
//! v0.2 collapses the v0.1 fan-out (markdown + kuzu + lancedb + sqlite FTS)
//! into a single transaction against `comemory.db`. Markdown stays the
//! source of truth: it is written first via [`MemoryStore::save`], then the
//! SQLite mirror is upserted in one transaction (memories + memory_tags +
//! memory_fts + optional memory_vec + edges).
//!
//! Embeddings are caller-supplied (BYO-vector). A vector may be passed as a
//! comma-separated list via `--vector` or as a JSON object on stdin via
//! `--vector-stdin`. The dim is validated against `schema_meta` before any
//! INSERT runs so wrong-dim payloads fail loudly.

use std::io::Read;
use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::embedding_input;
use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::{Kind, MemoryStore};
use crate::output::tty;
use crate::prelude::*;
use crate::store::{connection, embed, memory_row, vector};

const EXAMPLES: &str = "\
Examples:
  # Save a decision with tags and elevated quality
  comemory save \"Use Postgres for analytics\" --kind decision --repo myrepo --tags db,postgres --quality 4

  # Pipe a bug report body from another command
  echo \"Race in run_migration when run twice in <1s\" | comemory save - --kind bug --repo myrepo

  # Save with a caller-supplied embedding (BYO-vector)
  echo '{\"embedding\":[0.1,0.2,...]}' | comemory save \"...body...\" --vector-stdin

  # Minimal note (kind defaults to `note`, no repo/tags)
  comemory save \"Remember: cargo nextest serializes the embedder group\"";

/// Arguments to `comemory save`. The positional `body` is optional — if omitted
/// or `-`, the body is read from stdin so callers can pipe content.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Memory body. Use `-` (or omit) to read from stdin.
    pub body: Option<String>,
    /// Memory kind: decision|bug|convention|discovery|pattern|note.
    #[arg(long, value_enum, default_value_t = Kind::Note)]
    pub kind: Kind,
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
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=5))]
    pub quality: u8,
    /// Caller-supplied dense vector as a comma-separated float list. Length
    /// must equal the configured memory vector dim or the save fails with
    /// `vector dim mismatch`.
    #[arg(long)]
    pub vector: Option<String>,
    /// Read a JSON `{ "embedding": [..] }` payload from stdin and use it as
    /// the dense vector for the saved memory. Mutually exclusive with body
    /// being read from stdin (the body must be supplied as a positional arg
    /// when `--vector-stdin` is set).
    #[arg(long, default_value_t = false)]
    pub vector_stdin: bool,
}

/// JSON shape emitted under `--json`. `duplicate_of` is present only when a
/// live memory with a near-identical body (SimHash Hamming distance within
/// [`crate::simhash::NEAR_DUP_HAMMING`]) already exists — the save still
/// proceeds; the caller decides whether to mark it `supersedes`.
#[derive(Serialize)]
struct Output {
    id: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duplicate_of: Option<String>,
}

/// Save the body and emit the new memory id + on-disk path.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let body = match a.body.as_deref() {
        Some("-") | None => {
            if a.vector_stdin {
                return Err(Error::Config(
                    "--vector-stdin requires the body to be passed as a positional arg".into(),
                ));
            }
            read_stdin()?
        }
        Some(s) => s.to_string(),
    };
    // Trim, drop empties, and de-duplicate while preserving first-mention
    // order. `memory_tags` has a `PRIMARY KEY (memory_id, tag)` constraint,
    // so feeding `--tags foo,foo` straight through would abort the save
    // transaction with a UNIQUE violation. Empty entries (`--tags ,foo`)
    // are dropped because tag rows must be non-empty per the schema.
    let tags: Vec<String> = if a.tags.is_empty() {
        Vec::new()
    } else {
        let mut seen = std::collections::HashSet::new();
        a.tags
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty() && seen.insert(t.clone()))
            .collect()
    };

    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;

    // Parse the optional caller-supplied vector (CSV or JSON-on-stdin).
    let vector_opt = embedding_input::read_optional(a.vector_stdin, a.vector.as_deref())?;

    // Validate the vector's dim against the schema-locked memory dim BEFORE
    // the markdown write so a wrong-dim payload aborts cleanly with nothing
    // on disk. We open the DB connection here so the same handle is reused
    // by `write_sqlite_mirror` for the transactional mirror upsert below;
    // the dim guard runs against `schema_meta.memory_vector_dim` via
    // `vector::dim_memory`.
    let mut conn = connection::open(paths.db_path())?;
    if let Some(v) = vector_opt.as_deref() {
        let dim = vector::dim_memory(&conn)?;
        embed::guard_dim(v, dim)?;
    }

    // Near-duplicate check BEFORE the markdown write. Best-effort and
    // advisory only: the save always proceeds, the caller decides whether
    // to supersede the hit.
    let duplicate_of = near_duplicate(&conn, &body);

    // 1. Markdown atomic write (source of truth).
    let store = MemoryStore::new(paths.clone());
    let rec = store.save(&body, a.kind, &a.repo, &tags, &a.author, a.quality)?;

    // A re-save of an identical body produces the same content-hash-derived
    // id, so the pre-save scan would otherwise report the memory as a
    // duplicate of itself. Self-matches are not actionable — drop them.
    let duplicate_of = duplicate_of.filter(|dup| *dup != rec.frontmatter.id);

    // 2. SQLite mirror in one transaction. Markdown is the source of truth,
    //    so a mirror failure surfaces as an `Err` — but the markdown file
    //    is already on disk and can be replayed by `comemory rebuild`.
    //    We wrap the error with the markdown path and a rebuild hint so the
    //    operator knows exactly which file was written and how to reconcile.
    let md_path = rec.path.clone();
    write_sqlite_mirror(&mut conn, &rec, &tags, vector_opt.as_deref()).map_err(|e| {
        Error::Other(format!(
            "save: markdown at {} was written but SQLite mirror failed: {}; \
             run `comemory rebuild` to reconcile",
            md_path.display(),
            e
        ))
    })?;

    let output = Output {
        id: rec.frontmatter.id.clone(),
        path: rec.path.to_string_lossy().into_owned(),
        duplicate_of,
    };
    let mut out = std::io::stdout().lock();
    if json {
        writeln!(out, "{}", serde_json::to_string(&output)?)?;
    } else {
        writeln!(out, "saved {}", output.id)?;
        writeln!(out, "  path: {}", output.path)?;
        if let Some(dup) = output.duplicate_of.as_deref() {
            tty::warning(&format!(
                "similar memory {dup} exists — consider supersedes"
            ))?;
        }
    }
    Ok(())
}

/// Find a live memory whose body simhash is within near-dup range
/// ([`crate::simhash::NEAR_DUP_HAMMING`]) of `body`, returning the closest
/// hit's id. Best-effort: any DB error is logged and treated as "no
/// duplicate" so the check can never block a save. A full scan over live
/// rows is fine at personal-memory scale.
fn near_duplicate(conn: &rusqlite::Connection, body: &str) -> Option<String> {
    let hash = crate::simhash::simhash64(crate::simhash::tokens(body));
    let result: Result<Option<String>> = (|| {
        let mut stmt = conn.prepare("SELECT id, simhash FROM memories WHERE deleted_at IS NULL")?;
        let rows: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows
            .into_iter()
            .map(|(id, h)| (id, crate::simhash::hamming64(hash, h as u64)))
            .filter(|(_, d)| *d <= crate::simhash::NEAR_DUP_HAMMING)
            .min_by_key(|(_, d)| *d)
            .map(|(id, _)| id))
    })();
    match result {
        Ok(hit) => hit,
        Err(e) => {
            tracing::warn!(error = %e, "duplicate check skipped");
            None // dup check is best-effort: never blocks a save
        }
    }
}

/// Mirror the markdown record into `comemory.db` in a single transaction:
/// `memories`, `memory_tags`, `memory_fts`, optional `memory_vec`, and the
/// graph `edges` table (in_repo/authored_by/tagged plus cross-link
/// references parsed from the body). The caller-owned connection is passed
/// in so `run` can share the same handle used for the up-front
/// `vector::dim_memory` guard.
///
/// The non-vector branch is delegated to [`memory_row::insert`] so save and
/// `comemory rebuild` cannot drift on the row, tag, FTS, or edge SQL.
fn write_sqlite_mirror(
    conn: &mut rusqlite::Connection,
    rec: &crate::memory::MemoryRecord,
    tags: &[String],
    vector_opt: Option<&[f32]>,
) -> Result<()> {
    let tx = conn.transaction()?;
    let fm = &rec.frontmatter;
    let md_path = rec.path.to_string_lossy();
    memory_row::insert(&tx, fm, &rec.body, rec.slug.as_str(), &md_path, tags)?;
    if let Some(v) = vector_opt {
        // memory_vec is a vec0 vtab — its PK is `memory_id` but it does not
        // participate in SQLite's FK cascade, so a re-save of the same id
        // must drop any prior vector row before re-inserting or vec0 will
        // reject the second INSERT with a PK constraint failure.
        tx.execute(
            "DELETE FROM memory_vec WHERE memory_id = ?1",
            rusqlite::params![&fm.id],
        )?;
        vector::insert_memory(&tx, &fm.id, v)?;
    }
    tx.commit()?;
    Ok(())
}

fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}
