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
use serde::{Deserialize, Serialize};

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::cross_link;
use crate::graph::edges::{self, EdgeKey};
use crate::memory::{Kind, MemoryStore};
use crate::prelude::*;
use crate::store::{connection, embed, fts, vector};

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

/// JSON shape emitted under `--json`.
#[derive(Serialize)]
struct Output {
    id: String,
    path: String,
}

#[derive(Deserialize)]
struct EmbeddingPayload {
    embedding: Vec<f32>,
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
    let tags: Vec<String> = if a.tags.is_empty() {
        Vec::new()
    } else {
        a.tags.split(',').map(|t| t.trim().to_string()).collect()
    };

    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;

    // Parse the optional caller-supplied vector (CSV or JSON-on-stdin).
    let vector_opt = read_optional_vector(&a)?;

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

    // 1. Markdown atomic write (source of truth).
    let store = MemoryStore::new(paths.clone());
    let rec = store.save(&body, a.kind, &a.repo, &tags, &a.author, a.quality)?;

    // 2. SQLite mirror in one transaction. Markdown is the source of truth,
    //    so a mirror failure surfaces as an `Err` — but the markdown file
    //    is already on disk and can be replayed by `comemory rebuild`.
    write_sqlite_mirror(&mut conn, &rec, &tags, vector_opt.as_deref())?;

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

/// Read the optional caller-supplied vector from `--vector` (CSV) or
/// `--vector-stdin` (JSON `{ "embedding": [..] }`). Returns `Ok(None)` when
/// neither flag is set so the FTS-only path can proceed.
fn read_optional_vector(args: &Args) -> Result<Option<Vec<f32>>> {
    if args.vector_stdin {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(Error::Io)?;
        let payload: EmbeddingPayload = serde_json::from_str(buf.trim())?;
        return Ok(Some(payload.embedding));
    }
    if let Some(raw) = &args.vector {
        let parsed: Vec<f32> = raw
            .split(',')
            .map(|s| s.trim().parse::<f32>())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Config(format!("--vector parse: {e}")))?;
        return Ok(Some(parsed));
    }
    Ok(None)
}

/// Mirror the markdown record into `comemory.db` in a single transaction:
/// `memories`, `memory_tags`, `memory_fts`, optional `memory_vec`, and the
/// graph `edges` table (in_repo/authored_by/tagged plus cross-link
/// references parsed from the body). The caller-owned connection is passed
/// in so `run` can share the same handle used for the up-front
/// `vector::dim_memory` guard.
fn write_sqlite_mirror(
    conn: &mut rusqlite::Connection,
    rec: &crate::memory::MemoryRecord,
    tags: &[String],
    vector_opt: Option<&[f32]>,
) -> Result<()> {
    let tx = conn.transaction()?;

    let fm = &rec.frontmatter;
    let created_iso = format_iso(fm.created)?;
    let slug = rec.slug.as_str();
    let md_path = rec.path.to_string_lossy().to_string();

    let repo_opt: Option<&str> = if fm.repo.is_empty() {
        None
    } else {
        Some(fm.repo.as_str())
    };
    let author_opt: Option<&str> = if fm.author.is_empty() {
        None
    } else {
        Some(fm.author.as_str())
    };

    tx.execute(
        "INSERT INTO memories(\
             id, slug, kind, repo, author, quality, schema, \
             content_hash, body, created_at, updated_at, md_path) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        rusqlite::params![
            &fm.id,
            slug,
            fm.kind.as_str(),
            repo_opt,
            author_opt,
            fm.quality as i64,
            fm.schema as i64,
            &fm.content_hash,
            &rec.body,
            &created_iso,
            &created_iso,
            &md_path,
        ],
    )?;
    for tag in tags {
        tx.execute(
            "INSERT INTO memory_tags(memory_id, tag) VALUES(?1, ?2)",
            rusqlite::params![&fm.id, tag],
        )?;
    }
    fts::index_memory(&tx, &fm.id, &rec.body, &tags.join(","))?;

    if let Some(v) = vector_opt {
        vector::insert_memory(&tx, &fm.id, v)?;
    }

    insert_save_edges(&tx, fm, tags, &rec.body)?;
    tx.commit()?;
    Ok(())
}

/// Insert the v0.2 graph edges that accompany a save: in_repo, authored_by,
/// tagged, plus any references_file / references_symbol harvested from the
/// body by the cross-link parser.
fn insert_save_edges(
    tx: &rusqlite::Connection,
    fm: &crate::memory::Frontmatter,
    tags: &[String],
    body: &str,
) -> Result<()> {
    if !fm.repo.is_empty() {
        edges::insert(
            tx,
            EdgeKey {
                src_kind: "memory",
                src_id: &fm.id,
                dst_kind: "repo",
                dst_id: &fm.repo,
                rel: "in_repo",
            },
        )?;
    }
    if !fm.author.is_empty() {
        edges::insert(
            tx,
            EdgeKey {
                src_kind: "memory",
                src_id: &fm.id,
                dst_kind: "author",
                dst_id: &fm.author,
                rel: "authored_by",
            },
        )?;
    }
    for tag in tags {
        edges::insert(
            tx,
            EdgeKey {
                src_kind: "memory",
                src_id: &fm.id,
                dst_kind: "tag",
                dst_id: tag,
                rel: "tagged",
            },
        )?;
    }
    cross_link::extract_and_emit(tx, &fm.id, body)?;
    Ok(())
}

fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

/// Format an `OffsetDateTime` as RFC3339 / ISO-8601 for storage in the
/// `memories.created_at` / `updated_at` columns.
fn format_iso(t: time::OffsetDateTime) -> Result<String> {
    t.format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("iso8601 format: {e}")))
}
