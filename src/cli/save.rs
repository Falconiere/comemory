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
use crate::cli::{csv_unique, load_config, parse_id_csv, resolve_data_dir};
use crate::config::paths::Paths;
use crate::memory::{Kind, MemoryStore, Relations, SaveParams, id};
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
  comemory save \"Remember: cargo nextest serializes the embedder group\"

  # Replace an outdated memory: a1b2c3d4 is annotated `superseded_by` in
  # search results and demoted in ranking (score_parts.supersede = 0.2)
  comemory save \"new convention: pgbouncer in transaction mode\" --supersedes a1b2c3d4

  # Near-duplicate detection: if a similar memory exists, a TTY warning is
  # printed to stderr and --json output includes a `duplicate_of` field with
  # the matching memory id. The save always proceeds — use `--supersedes` to
  # mark the relationship if the new memory replaces the old one.";

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
    /// Comma-separated 8-hex memory ids this memory replaces (e.g.
    /// `a1b2c3d4,e5f6a7b8`). Recorded in the frontmatter
    /// `relations.supersedes` list and materialized as `supersedes` edges,
    /// so the older memories are demoted in ranking and annotated
    /// `superseded_by` in search results.
    #[arg(long, default_value = "")]
    pub supersedes: String,
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
/// `cfg.rank.near_dup_hamming`) already exists — the save still
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
    let tags = csv_unique(&a.tags);
    // The id is content-derived, so it is known before anything is written.
    // Computing it up front lets `--supersedes` reject a self-reference and
    // lets the near-dup scan exclude the body's own row on identical
    // re-saves (so the *second*-closest live near-dup still surfaces).
    let new_id = id::memory_id(&body);
    // Validate `--supersedes` BEFORE anything touches disk so a malformed
    // (or self-referential) id list aborts with no markdown file and no DB
    // rows.
    let supersedes = parse_supersedes(&a.supersedes, &new_id)?;

    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let cfg = load_config(&paths)?;

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
    let duplicate_of = near_duplicate(&conn, &body, &new_id, cfg.rank.near_dup_hamming);

    // 1. Markdown atomic write (source of truth). The `--supersedes` ids
    //    land in frontmatter `relations.supersedes`, so the relationship
    //    survives a `comemory rebuild` from markdown alone.
    let store = MemoryStore::new(paths.clone());
    let rec = store.save(SaveParams {
        body: &body,
        kind: a.kind,
        repo: &a.repo,
        tags: &tags,
        author: &a.author,
        quality: a.quality,
        relations: Relations {
            supersedes,
            ..Relations::default()
        },
    })?;

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

/// Find a live memory whose body simhash is within `radius` Hamming bits
/// of `body` (callers pass `cfg.rank.near_dup_hamming`, which defaults to
/// [`crate::simhash::NEAR_DUP_HAMMING`]), returning the closest hit's id.
/// `self_id` (the body's own content-derived id) is excluded before the
/// closest-hit selection so an identical re-save still surfaces the
/// second-closest live near-dup instead of matching itself. Best-effort:
/// any DB error is logged and treated as "no duplicate" so the check can
/// never block a save. A full scan over live rows is fine at
/// personal-memory scale.
fn near_duplicate(
    conn: &rusqlite::Connection,
    body: &str,
    self_id: &str,
    radius: u32,
) -> Option<String> {
    let hash = crate::simhash::of_body(body);
    match near_duplicate_inner(conn, hash, self_id, radius) {
        Ok(hit) => hit,
        Err(e) => {
            tracing::warn!(error = %e, "duplicate check skipped");
            None // dup check is best-effort: never blocks a save
        }
    }
}

/// Fallible core of [`near_duplicate`]: scan live `memories` rows (minus
/// the body's own `self_id` row) and return the id of the closest simhash
/// neighbor within `radius` Hamming bits, if any.
fn near_duplicate_inner(
    conn: &rusqlite::Connection,
    hash: u64,
    self_id: &str,
    radius: u32,
) -> Result<Option<String>> {
    let mut stmt =
        conn.prepare("SELECT id, simhash FROM memories WHERE deleted_at IS NULL AND id <> ?1")?;
    let rows: Vec<(String, i64)> = stmt
        .query_map([self_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(id, h)| (id, crate::simhash::hamming64(hash, h as u64)))
        .filter(|(_, d)| *d <= radius)
        .min_by_key(|(_, d)| *d)
        .map(|(id, _)| id))
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

/// Parse and validate the `--supersedes` CSV via the shared
/// [`parse_id_csv`]: every entry must be a well-formed 8-hex-lowercase
/// memory id and must not equal `self_id` (the content-derived id of the
/// body being saved) — a memory cannot supersede itself, and a self-edge
/// would permanently penalize the memory in ranking and flag it for
/// prune. The target memory is *not* required to exist — edges may dangle
/// (same stance as cross-link refs) and every consumer JOINs on live
/// `memories` rows.
fn parse_supersedes(raw: &str, self_id: &str) -> Result<Vec<String>> {
    let ids = parse_id_csv(raw, "--supersedes")?;
    for entry in &ids {
        if entry == self_id {
            return Err(Error::Config(format!(
                "--supersedes: a memory cannot supersede itself (`{entry}` is the id of the \
                 body being saved)"
            )));
        }
    }
    Ok(ids)
}

fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}
