//! `api::save::{Request, Response, run}` — the shared middle of `comemory
//! save` / `POST /api/v1/memories`, moved out of `cli::save::run` (Binding
//! Rule 1): id derivation, `supersedes` validation, `ref_*` collection,
//! vector dim guard, near-dup check, atomic markdown write + SQLite mirror.
//!
//! **cwd semantics.** `ref_file`/`ref_symbol` anchoring resolves the git
//! working-tree root via `git2::Repository::discover(cwd)`. Over HTTP this
//! is the **server process's** cwd, not the client's — documented API
//! behavior (spec §Architecture); deterministic anchoring needs explicit
//! `repo:`-qualified `ref_file`/`ref_symbol` values.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::cli::{parse_id_csv, ref_args};
use crate::memory::{Kind, MemoryStore, References, Relations, SaveParams, id};
use crate::prelude::*;
use crate::store::{embed, memory_row, vector};

/// `comemory save` / `POST /api/v1/memories` request. The stdin/`-` body
/// convenience is CLI-only — `body` is a required JSON field over HTTP.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Memory body (markdown).
    pub body: String,
    /// Memory kind: decision|bug|convention|discovery|pattern|note.
    #[serde(default = "default_kind")]
    pub kind: Kind,
    /// Repo name attached to the memory (free-form string).
    #[serde(default)]
    pub repo: String,
    /// Tag list. Already de-duplicated is the caller's responsibility
    /// (`store::memory_row::insert` de-dupes defense-in-depth regardless).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Author identifier.
    #[serde(default)]
    pub author: String,
    /// Quality rating 1..=5. Defaults to 3. The CLI enforces this range via
    /// a clap validator before `run` is ever called; HTTP has no clap, so
    /// [`run`] validates it explicitly.
    #[serde(default = "default_quality")]
    pub quality: u8,
    /// 8-hex memory ids this memory replaces. Materialized as `supersedes`
    /// edges; the target memories are demoted in ranking and annotated
    /// `superseded_by` in search results.
    #[serde(default)]
    pub supersedes: Vec<String>,
    /// Caller-supplied dense vector, replacing `--vector`/`--vector-stdin`.
    /// Length must equal the configured memory vector dim or the save fails
    /// with `vec_dim_mismatch`.
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    /// Version-anchored file references (`[repo:]path`).
    #[serde(default)]
    pub ref_file: Vec<String>,
    /// Version-anchored symbol references (`[repo:]path:symbol`).
    #[serde(default)]
    pub ref_symbol: Vec<String>,
}

fn default_kind() -> Kind {
    Kind::Note
}

fn default_quality() -> u8 {
    3
}

/// `comemory save` / `POST /api/v1/memories` response.
#[derive(Serialize, Debug)]
pub struct Response {
    /// 8-hex content-derived memory id.
    pub id: String,
    /// On-disk path of the written markdown file.
    pub path: String,
    /// Present only when a live near-duplicate memory was found (SimHash
    /// Hamming distance within `cfg.rank.near_dup_hamming`). The save
    /// always proceeds; the caller decides whether to re-save with
    /// `supersedes`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
    /// Version-pointer ref advisories (untracked / cross-repo refs saved
    /// unpinned). Omitted when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Save the body and return the new memory id + on-disk path. See the
/// module doc for the HTTP cwd-anchoring caveat.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    validate_quality(req.quality)?;
    let cfg = ctx.cfg;
    let paths = ctx.paths;
    // Content-derived id is known before any write, so `supersedes` and the
    // near-dup scan can use it up front.
    let new_id = id::memory_id(&req.body);
    // Validate `supersedes` and `ref_*` BEFORE touching disk: a malformed
    // value aborts with no markdown file and no DB rows.
    let supersedes = validate_supersedes(&req.supersedes, &new_id)?;
    let (references, ref_warnings) = collect_refs(&req)?;

    // Validation is now behind us — safe to touch the filesystem/DB. Mirrors
    // `cli::save::run`'s original ordering exactly (AC-13): a malformed
    // `supersedes`/`ref_*` value never creates the data dir or opens the DB.
    paths.ensure_dirs()?;
    let conn = ctx.conn()?;
    if let Some(v) = req.vector.as_deref() {
        let dim = vector::dim_memory(conn)?;
        embed::guard_dim(v, dim)?;
    }
    let duplicate_of = near_duplicate(conn, &req.body, &new_id, cfg.rank.near_dup_hamming);

    let params = build_params(&req, supersedes, references);
    let rec = persist(conn, paths, params, req.vector.as_deref())?;

    Ok(Response {
        id: rec.frontmatter.id.clone(),
        path: rec.path.to_string_lossy().into_owned(),
        duplicate_of,
        warnings: ref_warnings,
    })
}

/// `1..=5`, matching the CLI's clap range validator (HTTP has no clap, so
/// this must be enforced explicitly).
fn validate_quality(quality: u8) -> Result<()> {
    if (1..=5).contains(&quality) {
        Ok(())
    } else {
        Err(Error::BadRequest(format!(
            "quality must be in 1..=5, got {quality}"
        )))
    }
}

/// Collect `ref_file`/`ref_symbol` against the discovered repo root. See the
/// module doc for the HTTP cwd caveat.
fn collect_refs(req: &Request) -> Result<(References, Vec<String>)> {
    let repo_root = resolve_repo_root();
    ref_args::collect(
        &req.ref_file,
        &req.ref_symbol,
        &req.repo,
        repo_root.as_deref(),
    )
}

/// Assemble the [`SaveParams`] the store layer expects.
fn build_params<'a>(
    req: &'a Request,
    supersedes: Vec<String>,
    references: References,
) -> SaveParams<'a> {
    SaveParams {
        body: &req.body,
        kind: req.kind,
        repo: &req.repo,
        tags: &req.tags,
        author: &req.author,
        quality: req.quality,
        relations: Relations {
            supersedes,
            ..Relations::default()
        },
        references,
    }
}

/// Write the markdown record (source of truth), then mirror it into
/// `comemory.db` in one transaction. A mirror failure keeps the markdown and
/// names it plus the `rebuild` recovery path.
fn persist(
    conn: &mut rusqlite::Connection,
    paths: &crate::config::Paths,
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
    crate::graph::derived::refresh_derived_best_effort(conn);
    Ok(rec)
}

/// Discover the git working-tree root containing the process cwd, or `None`
/// when not run inside a repo. See the module doc for the HTTP caveat: this
/// resolves the *server process's* cwd over HTTP.
fn resolve_repo_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let repo = git2::Repository::discover(&cwd).ok()?;
    repo.workdir().map(Path::to_path_buf)
}

/// Find a live memory whose body simhash is within `radius` Hamming bits of
/// `body`, returning the closest hit's id. `self_id` (the body's own
/// content-derived id) is excluded first so an identical re-save still
/// surfaces the second-closest live near-dup instead of matching itself.
/// Best-effort: any DB error is logged and treated as "no duplicate" so the
/// check can never block a save.
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
/// `self_id`) and return the id of the closest simhash neighbor within
/// `radius` Hamming bits, if any.
fn near_duplicate_inner(
    conn: &rusqlite::Connection,
    hash: u64,
    self_id: &str,
    radius: u32,
) -> Result<Option<String>> {
    Ok(
        crate::store::simhash_scan::live_simhashes(conn, None, Some(self_id))?
            .into_iter()
            .map(|row| (row.id, crate::simhash::hamming64(hash, row.simhash as u64)))
            .filter(|(_, d)| *d <= radius)
            .min_by_key(|(_, d)| *d)
            .map(|(id, _)| id),
    )
}

/// Mirror the markdown record into `comemory.db` in a single transaction:
/// `memories`, `memory_tags`, `memory_fts`, optional `memory_vec`, and the
/// graph `edges` table. The non-vector branch is delegated to
/// [`memory_row::insert`] so save and `comemory rebuild` cannot drift.
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
        // memory_vec is a vec0 vtab whose PK does not participate in SQLite's
        // FK cascade, so a re-save of the same id must drop any prior vector
        // row before re-inserting.
        tx.execute(
            "DELETE FROM memory_vec WHERE memory_id = ?1",
            rusqlite::params![&fm.id],
        )?;
        vector::insert_memory(&tx, &fm.id, v)?;
    }
    tx.commit()?;
    Ok(())
}

/// Validate `raw` (the `supersedes` field) via the shared [`parse_id_csv`] —
/// joined back into the CSV shape it parses, so this and
/// `cli::save::Args::supersedes` cannot drift on what counts as a valid id —
/// then reject any entry equal to `self_id` (a memory cannot supersede
/// itself). Targets are not required to exist; edges may dangle, same as
/// cross-link refs.
///
/// The `--supersedes` flag name (not the JSON field name `supersedes`) is
/// passed through to [`parse_id_csv`] so the error text matches
/// `cli::save`'s original `parse_supersedes` byte-for-byte (AC-13); the
/// same message reaches HTTP callers, referencing the CLI flag rather than
/// the JSON field — a known, accepted rough edge.
fn validate_supersedes(raw: &[String], self_id: &str) -> Result<Vec<String>> {
    let ids = parse_id_csv(&raw.join(","), "--supersedes")?;
    for entry in &ids {
        if entry == self_id {
            // `Error::Config` (not `BadRequest`) matches `cli::save`'s
            // original `parse_supersedes` exactly — same exit code (78,
            // AC-13) and, over HTTP, the same 400 `code:"config"` either
            // variant would map to.
            return Err(Error::Config(format!(
                "--supersedes: a memory cannot supersede itself (`{entry}` is the id of the \
                 body being saved)"
            )));
        }
    }
    Ok(ids)
}
