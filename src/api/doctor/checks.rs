//! The individual `comemory doctor` health probes plus [`run_all`], which
//! runs every one of them against an already-open connection and returns
//! the ordered [`Check`] list alongside every scalar the checks derive
//! ([`Extras`]). Shared by `super::writable_report` and
//! `super::forward_compat_report` — both only ever read `conn`, so the same
//! pass works whether it is the primary read-write connection or the
//! forward-compat read-only fallback (see the parent module doc).

use rusqlite::Connection;

use crate::config::Paths;
use crate::config::env::env_parse;
use crate::memory::MemoryStore;
use crate::memory::id::sha256_hex;
use crate::prelude::*;

/// One named health probe inside [`super::Report::checks`]. `status` is a
/// plain string (`"ok"` | `"warn"` | `"fail"`) rather than an enum so the
/// JSON shape needs no custom (de)serializer and a future status value
/// never forces a schema bump.
#[derive(serde::Serialize, Debug, Clone)]
pub struct Check {
    /// Stable, human-readable probe name (e.g. `"mirror parity"`).
    pub name: String,
    /// `"ok"` | `"warn"` | `"fail"`.
    pub status: String,
    /// One-line explanation of the result.
    pub detail: String,
    /// The `/api/v1` route a console can call to fix a `warn`/`fail`
    /// (e.g. `"POST /api/v1/rebuild"`), or `None` when the remedy is not
    /// an API call (upgrade the binary, set an env var).
    pub remedy: Option<String>,
}

impl Check {
    /// Attach the remedy route for a non-`ok` result; an `ok` check keeps
    /// `None` — there is nothing to fix.
    fn with_remedy(mut self, remedy: &str) -> Self {
        if self.status != "ok" {
            self.remedy = Some(remedy.to_string());
        }
        self
    }
}

/// Build a [`Check`] with the given `status`.
fn check(name: &str, status: &str, detail: impl Into<String>) -> Check {
    Check {
        name: name.to_string(),
        status: status.to_string(),
        detail: detail.into(),
        remedy: None,
    }
}

/// Build a passing [`Check`].
pub(crate) fn ok(name: &str, detail: impl Into<String>) -> Check {
    check(name, "ok", detail)
}

/// Build an advisory, non-fatal [`Check`].
fn warn(name: &str, detail: impl Into<String>) -> Check {
    check(name, "warn", detail)
}

/// Build a failing [`Check`].
fn fail(name: &str, detail: impl Into<String>) -> Check {
    check(name, "fail", detail)
}

/// Every scalar the check pipeline derives, alongside the ordered [`Check`]
/// list itself — bundled so [`run_all`] returns one value instead of a wide
/// tuple, and `doctor.rs`'s `assemble` merges it onto the original six
/// scalar fields to build a [`super::Report`].
#[derive(Default)]
pub(crate) struct Extras {
    /// Every probe [`run_all`] ran, in a fixed order.
    pub checks: Vec<Check>,
    /// `comemory.db`'s file size in bytes.
    pub db_bytes: u64,
    /// Live (non-trashed) markdown files under `memories/`.
    pub markdown_files: u64,
    /// Markdown files whose `sha256(body.trim_end())` disagrees with (or
    /// has no) `memories.content_hash` row.
    pub mirror_drift: u64,
    /// `memory_vec`'s configured dimension, read from `schema_meta`.
    pub memory_vec_dim: Option<u32>,
    /// `code_vec`'s configured dimension, read from `schema_meta`.
    pub code_vec_dim: Option<u32>,
    /// Path to the newest `comemory.db.pre-v{N}.bak` snapshot, if any.
    pub backup_path: Option<String>,
    /// That snapshot's file size in bytes.
    pub backup_bytes: Option<u64>,
    /// Whether the FTS5 `identifier` tokenizer registered on `conn`.
    pub tokenizer_registered: bool,
    /// `repo_marker.root_path` entries that still exist on disk.
    pub repo_roots_ok: u32,
    /// `repo_marker.root_path` entries recorded at all (non-`NULL`).
    pub repo_roots_total: u32,
    /// The configured `COMEMORY_EMBED_CMD`, if set.
    pub embed_cmd: Option<String>,
    /// How long the embed probe took, when it succeeded.
    pub embed_probe_ms: Option<u64>,
}

impl Extras {
    /// Degraded extras for `super::unwritable_report` — no connection was
    /// ever opened, so only the failing "data dir writable" check is real;
    /// every DB-derived field reports its zero/`None` default.
    pub(crate) fn unwritable() -> Self {
        Self {
            checks: vec![data_dir_writable(false)],
            ..Self::default()
        }
    }
}

/// Run every doctor probe against `conn`, returning the ordered [`Check`]
/// list plus every scalar they derive.
pub(crate) fn run_all(conn: &Connection, paths: &Paths, schema_version: &str) -> Result<Extras> {
    let mut checks = Vec::with_capacity(10);
    checks.push(data_dir_writable(true));

    let (mirror_check, markdown_files, mirror_drift) = mirror_parity(conn, paths)?;
    checks.push(mirror_check);
    checks.push(schema_version_check(
        schema_version,
        crate::store::migrate::CURRENT_VERSION,
    ));

    let (backup_check, backup_path, backup_bytes) = super::backup::migration_backup(paths)?;
    checks.push(backup_check);

    let (tokenizer_check, tokenizer_registered) = tokenizer(conn);
    checks.push(tokenizer_check);

    let sqlite_vec_loaded = conn
        .query_row("SELECT vec_version()", [], |r| r.get::<_, String>(0))
        .is_ok();
    let (vec_check, memory_vec_dim, code_vec_dim) = vector_dims(conn, sqlite_vec_loaded);
    checks.push(vec_check);

    let (repo_check, repo_roots_ok, repo_roots_total) = repo_roots(conn)?;
    checks.push(repo_check);

    let (embed_cmd, embed_probe_ms) =
        push_embed_and_counts(conn, paths, markdown_files, &mut checks)?;

    Ok(Extras {
        checks,
        db_bytes: db_file_size(paths),
        markdown_files,
        mirror_drift,
        memory_vec_dim,
        code_vec_dim,
        backup_path,
        backup_bytes,
        tokenizer_registered,
        repo_roots_ok,
        repo_roots_total,
        embed_cmd,
        embed_probe_ms,
    })
}

/// Checks 8-10 (embed probe, markdown/db counts, data-dir layout), pushed
/// onto the caller's `checks` list — split out of [`run_all`] to keep it
/// under the function-length ceiling. Returns the embed scalars, the only
/// two of the three checks' outputs [`Extras`] needs by name.
fn push_embed_and_counts(
    conn: &Connection,
    paths: &Paths,
    markdown_files: u64,
    checks: &mut Vec<Check>,
) -> Result<(Option<String>, Option<u64>)> {
    let embed_cmd = env_parse::<String>("COMEMORY_EMBED_CMD")?;
    let (embed_check, embed_probe_ms) = embed_probe(embed_cmd.as_deref());
    checks.push(embed_check);
    checks.push(markdown_db_counts(conn, markdown_files)?);
    checks.push(data_dir_layout(paths));
    Ok((embed_cmd, embed_probe_ms))
}

/// `comemory.db`'s file size in bytes, or `0` if it vanished between the
/// caller opening it and this read — not worth failing the whole doctor run
/// over a size we can live without.
pub(crate) fn db_file_size(paths: &Paths) -> u64 {
    std::fs::metadata(paths.db_path()).map_or(0, |m| m.len())
}

/// Check 1: is `comemory.db` (or the data dir, for a not-yet-created one)
/// writable. `writable` is the caller's own `probe_writable` result — this
/// is always `true` on the [`run_all`] path, since a `false` probe short
/// circuits to `Extras::unwritable` before a connection ever opens.
fn data_dir_writable(writable: bool) -> Check {
    if writable {
        ok("data dir writable", "comemory.db is writable")
    } else {
        fail("data dir writable", "not writable")
    }
}

/// Check 2: every live markdown file's `sha256(body.trim_end())` against
/// its `memories.content_hash` row — NOT mtime (`rebuild` rewrites rows and
/// `git checkout` rewrites mtimes, so an mtime check would report drift
/// that does not exist). Returns the check, the markdown file count, and
/// the drift count.
fn mirror_parity(conn: &Connection, paths: &Paths) -> Result<(Check, u64, u64)> {
    let store = MemoryStore::new(paths.clone());
    let records = store.list()?;
    let markdown_files = records.len() as u64;
    // ONE query for every stored hash, not one per markdown file: a corpus
    // is thousands of files and `doctor` runs on demand. The map is keyed by
    // memory id, so a file with no row at all reads as `None` and counts as
    // drift exactly like a mismatching hash does.
    let mut stmt =
        conn.prepare("SELECT id, content_hash FROM memories WHERE deleted_at IS NULL")?;
    let stored: std::collections::HashMap<String, String> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut drift = 0u64;
    for rec in &records {
        let hash = sha256_hex(rec.body.trim_end().as_bytes());
        if stored.get(&rec.frontmatter.id).map(String::as_str) != Some(hash.as_str()) {
            drift += 1;
        }
    }
    let result = if drift == 0 {
        ok("mirror parity", "no drift")
    } else {
        warn(
            "mirror parity",
            format!("{drift} markdown file(s) disagree with (or have no) memories row"),
        )
        .with_remedy("POST /api/v1/rebuild")
    };
    Ok((result, markdown_files, drift))
}

/// Check 3: the applied schema version matches this build's.
fn schema_version_check(version: &str, current: &str) -> Check {
    if version == current {
        ok("schema version", format!("at current version {current}"))
    } else {
        fail("schema version", format!("{version} != {current}"))
            .with_remedy("POST /api/v1/rebuild")
    }
}

/// The most-recently-modified file directly under `dir` whose name starts
/// with `prefix`, with its size.
pub(crate) fn newest_matching(
    dir: &std::path::Path,
    prefix: &str,
) -> Result<Option<(std::path::PathBuf, u64)>> {
    if !dir.is_dir() {
        return Ok(None);
    }
    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf, u64)> = None;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !name.starts_with(prefix) {
            continue;
        }
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified()?;
        let is_newer = match &newest {
            Some((t, _, _)) => modified > *t,
            None => true,
        };
        if is_newer {
            newest = Some((modified, entry.path(), meta.len()));
        }
    }
    Ok(newest.map(|(_, path, size)| (path, size)))
}

/// Check 5: the FTS5 `identifier` tokenizer (`src/store/tokenizer/`)
/// registers cleanly on `conn`. Re-registration is idempotent (see
/// `tokenizer::ffi::register`'s own doc), so this doubles as a direct
/// functional proof rather than an indirect FTS query.
fn tokenizer(conn: &Connection) -> (Check, bool) {
    match crate::store::tokenizer::ffi::register(conn) {
        Ok(()) => (ok("fts5 tokenizer", "registered"), true),
        Err(e) => (fail("fts5 tokenizer", format!("failed: {e}")), false),
    }
}

/// Check 6: `sqlite-vec` loaded, with the `memory_vec` / `code_vec` dims
/// read from `schema_meta` rather than hardcoded.
fn vector_dims(conn: &Connection, sqlite_vec_loaded: bool) -> (Check, Option<u32>, Option<u32>) {
    let memory_dim = crate::store::vector::dim_memory(conn)
        .ok()
        .and_then(|d| u32::try_from(d).ok());
    let code_dim = crate::store::vector::dim_code(conn)
        .ok()
        .and_then(|d| u32::try_from(d).ok());
    let detail = format!(
        "sqlite-vec loaded={sqlite_vec_loaded}, memory_vec dim={memory_dim:?}, \
         code_vec dim={code_dim:?}"
    );
    let result = if sqlite_vec_loaded && memory_dim.is_some() && code_dim.is_some() {
        ok("sqlite-vec", detail)
    } else {
        warn("sqlite-vec", detail)
    };
    (result, memory_dim, code_dim)
}

/// Check 7: `repo_marker.root_path` entries that still exist on disk. The
/// remedy names the repos that failed rather than a `{name}` template: the
/// operator reading a doctor report should be able to run the suggestion
/// as-is, and this check already knows which labels are unresolvable.
fn repo_roots(conn: &Connection) -> Result<(Check, u32, u32)> {
    let mut stmt =
        conn.prepare("SELECT repo, root_path FROM repo_marker WHERE root_path IS NOT NULL")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut ok_count = 0u32;
    let mut total = 0u32;
    let mut missing: Vec<String> = Vec::new();
    for row in rows {
        let (repo, root_path) = row?;
        total += 1;
        if std::path::Path::new(&root_path).exists() {
            ok_count += 1;
        } else {
            missing.push(repo);
        }
    }
    let detail = format!("{ok_count}/{total} repo root(s) resolvable");
    let result = if ok_count == total {
        ok("repo roots", detail)
    } else {
        archive_remedy(&missing).map_or_else(
            || warn("repo roots", detail.clone()),
            |remedy| warn("repo roots", detail.clone()).with_remedy(&remedy),
        )
    };
    Ok((result, ok_count, total))
}

/// The archive suggestion for every repo whose root has gone: one runnable
/// route per label, joined, so nothing is left for the reader to fill in.
///
/// Returns `None` for an empty list rather than a `{name}` template the
/// console cannot invoke — and the caller only reaches this on the
/// `ok_count < total` branch, where at least one label is missing by
/// construction, so `None` never actually reaches a report.
fn archive_remedy(missing: &[String]) -> Option<String> {
    if missing.is_empty() {
        return None;
    }
    Some(
        missing
            .iter()
            .map(|repo| format!("POST /api/v1/repos/{repo}/archive"))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// Check 8: run the configured `COMEMORY_EMBED_CMD` (`crate::embed`) and
/// time it. A missing command or a failing probe is `"warn"`, never a hard
/// error — see the module doc's "Forward-compat fallback" sibling
/// invariant: `doctor`'s job is to report a broken state, not become one.
fn embed_probe(cmd: Option<&str>) -> (Check, Option<u64>) {
    let Some(cmd) = cmd else {
        return (warn("embed command", "COMEMORY_EMBED_CMD is not set"), None);
    };
    let started = std::time::Instant::now();
    match crate::embed::embed_query(cmd, "comemory doctor embed probe") {
        Ok(vector) => {
            let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            (
                ok(
                    "embed command",
                    format!("probe returned a {}-dim vector", vector.len()),
                ),
                Some(elapsed),
            )
        }
        Err(e) => (warn("embed command", format!("probe failed: {e}")), None),
    }
}

/// Check 9: markdown file count vs `memories` row count.
fn markdown_db_counts(conn: &Connection, markdown_files: u64) -> Result<Check> {
    // LIVE rows only: `markdown_files` comes from `MemoryStore::list()`, which
    // never walks `.trash/`, so counting soft-deleted rows here would report a
    // permanent spurious warn on any corpus that has ever had a memory pruned.
    // `COUNT(*)` is an i64 to SQLite (rusqlite 0.40 dropped `FromSql for
    // u64`); it is never negative, so the fallback is unreachable.
    let db_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE deleted_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    let db_rows = u64::try_from(db_rows).unwrap_or(0);
    let detail = format!("{markdown_files} markdown file(s), {db_rows} memories row(s)");
    Ok(if markdown_files == db_rows {
        ok("markdown/db counts", detail)
    } else {
        warn("markdown/db counts", detail)
    })
}

/// Check 10: the expected data-dir layout (`memories/`, `comemory.db`) is
/// present.
fn data_dir_layout(paths: &Paths) -> Check {
    let memories_ok = paths.memories_dir().is_dir();
    let db_ok = paths.db_path().is_file();
    if memories_ok && db_ok {
        ok("data dir layout", "memories/ and comemory.db present")
    } else {
        warn(
            "data dir layout",
            format!("memories_dir_present={memories_ok}, db_present={db_ok}"),
        )
    }
}
