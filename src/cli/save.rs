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
use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::embedding_input;
use crate::cli::{csv_unique, load_config, parse_id_csv, ref_args, resolve_data_dir};
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
    /// Version-anchored file reference `[repo:]path` (repeatable;
    /// comma-splittable). Pins the HEAD-tree blob + commit + branch when the
    /// path is tracked in the cwd repo; untracked/cross-repo refs save
    /// unpinned with an advisory warning.
    #[arg(long)]
    pub ref_file: Vec<String>,
    /// Version-anchored symbol reference `[repo:]path:symbol` (repeatable;
    /// comma-splittable). A value without a trailing `:symbol` is a usage
    /// error (exit 64). Anchoring matches `--ref-file`.
    #[arg(long)]
    pub ref_symbol: Vec<String>,
}

/// JSON shape emitted under `--json`. `duplicate_of` is present only when a
/// live memory with a near-identical body (SimHash Hamming distance within
/// `cfg.rank.near_dup_hamming`) already exists — the save still
/// proceeds; the caller decides whether to mark it `supersedes`.
///
/// `warnings` collects the version-pointer ref advisories (untracked /
/// cross-repo refs saved unpinned); it is omitted from `--json` output when
/// empty, mirroring the `duplicate_of` advisory convention.
#[derive(Serialize)]
struct Output {
    id: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duplicate_of: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

/// Save the body and emit the new memory id + on-disk path.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let body = read_body(&a)?;
    let tags = csv_unique(&a.tags);
    // Content-derived id is known before any write, so `--supersedes` and the
    // near-dup scan can use it up front.
    let new_id = id::memory_id(&body);
    // Validate `--supersedes` and `--ref-*` BEFORE touching disk: a malformed
    // value aborts with no markdown file and no DB rows.
    let supersedes = parse_supersedes(&a.supersedes, &new_id)?;
    let repo_root = resolve_repo_root();
    let (references, ref_warnings) =
        ref_args::collect(&a.ref_file, &a.ref_symbol, &a.repo, repo_root.as_deref())?;

    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let cfg = load_config(&paths)?;

    // Validate the caller-supplied vector's dim before the write, reusing the
    // connection for the transactional mirror upsert below.
    let vector_opt = embedding_input::read_optional(a.vector_stdin, a.vector.as_deref())?;
    let mut conn = connection::open(paths.db_path())?;
    if let Some(v) = vector_opt.as_deref() {
        let dim = vector::dim_memory(&conn)?;
        embed::guard_dim(v, dim)?;
    }
    let duplicate_of = near_duplicate(&conn, &body, &new_id, cfg.rank.near_dup_hamming);

    let params = SaveParams {
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
        references,
    };
    let rec = persist(&mut conn, &paths, params, vector_opt.as_deref())?;

    let output = Output {
        id: rec.frontmatter.id.clone(),
        path: rec.path.to_string_lossy().into_owned(),
        duplicate_of,
        warnings: ref_warnings,
    };
    emit(json, &output)
}

/// Write the markdown record (source of truth, surviving `rebuild` because
/// `--supersedes` ids and `--ref-*` anchors live in the frontmatter), then
/// mirror it into `comemory.db` in one transaction. A mirror failure keeps
/// the markdown and names it plus the `rebuild` recovery path.
fn persist(
    conn: &mut rusqlite::Connection,
    paths: &Paths,
    params: SaveParams<'_>,
    vector_opt: Option<&[f32]>,
) -> Result<crate::memory::MemoryRecord> {
    let tags = params.tags.to_vec();
    let store = MemoryStore::new(paths.clone());
    let rec = store.save(params)?;
    let md_path = rec.path.clone();
    write_sqlite_mirror(conn, &rec, &tags, vector_opt).map_err(|e| {
        Error::Other(format!(
            "save: markdown at {} was written but SQLite mirror failed: {}; \
             run `comemory rebuild` to reconcile",
            md_path.display(),
            e
        ))
    })?;
    Ok(rec)
}

/// Resolve the body from the positional arg or stdin, rejecting the
/// `--vector-stdin` + stdin-body combination (both would consume stdin).
fn read_body(a: &Args) -> Result<String> {
    match a.body.as_deref() {
        Some("-") | None => {
            if a.vector_stdin {
                return Err(Error::Config(
                    "--vector-stdin requires the body to be passed as a positional arg".into(),
                ));
            }
            read_stdin()
        }
        Some(s) => Ok(s.to_string()),
    }
}

/// Discover the git working-tree root containing the process cwd, or `None`
/// when the save is not run inside a repo. Used to make `--ref-*` paths
/// repo-root-relative and to capture anchors.
fn resolve_repo_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let repo = git2::Repository::discover(&cwd).ok()?;
    repo.workdir().map(Path::to_path_buf)
}

/// Emit the save result: a single JSON object under `--json`, else a TTY
/// summary with the near-dup advisory and each ref warning on stderr.
fn emit(json: bool, output: &Output) -> Result<()> {
    let mut out = std::io::stdout().lock();
    if json {
        writeln!(out, "{}", serde_json::to_string(output)?)?;
        return Ok(());
    }
    writeln!(out, "saved {}", output.id)?;
    writeln!(out, "  path: {}", output.path)?;
    if let Some(dup) = output.duplicate_of.as_deref() {
        tty::warning(&format!(
            "similar memory {dup} exists — consider supersedes"
        ))?;
    }
    for w in &output.warnings {
        tty::warning(w)?;
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
