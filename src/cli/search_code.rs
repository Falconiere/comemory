//! `comemory search-code` — ranked search over the indexed `code_symbols`
//! table (BM25 + optional BYO-vector ANN, RRF-fused), reranked by the
//! PageRank / activation / working-set affinity / feedback priors.
//!
//! Mirrors `comemory search` (`crate::cli::search`): resolve the data dir,
//! open `comemory.db`, parse any caller-supplied vector, route via
//! [`crate::retrieval::code_route::route_code`], rerank via
//! [`crate::retrieval::code_rerank::rerank_code`], cut to `top_k`, record
//! telemetry, and emit. Code vectors are 768-dim (vs 1024 for memories);
//! the dim guard lives inside `store::vector::knn_code`, so a wrong-dim
//! vector fails there, not at parse time.
//!
//! ## Working-set affinity scope
//!
//! The affinity prior needs the repo's checkout path, which the database
//! does not know — `search-code` may run from anywhere. Decision: the
//! process CWD is used as the working-tree candidate, so the affinity
//! boost activates only when the command runs inside the relevant repo's
//! checkout (documented in `--help`). [`code_rerank::working_set`] is
//! best-effort by contract — a non-repo CWD degrades to the empty set and
//! a neutral prior. The working-set file ids need a repo *label* too:
//! the `--repo` filter when given, else the basename of the discovered
//! working tree (matching the common `index-code --repo <dirname>`
//! convention); when neither resolves, affinity stays neutral.

use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;
use rusqlite::Connection;
use time::OffsetDateTime;

use crate::ast::languages::{self, Lang};
use crate::cli::{embedding_input, load_config, override_top_k, resolve_data_dir};
use crate::config::paths::Paths;
use crate::output;
use crate::prelude::*;
use crate::retrieval::code_rerank::{self, CodeReranked, WorkingSet};
use crate::retrieval::code_route;
use crate::store::{connection, memory_row};

const EXAMPLES: &str = "\
Examples:
  # Lexical code search; identifier tokens split automatically
  comemory search-code \"parse frontmatter\"

  # JSON output; hits[].score_parts breaks down every ranking factor
  # (relevance, rank, activation, affinity, feedback, final_score) and
  # the envelope carries query_id — pass it to
  # `comemory feedback <query_id> --used-code <ids>`.
  comemory search-code \"dim guard\" --json

  # Scope to one repo and language (aliases like `rs`/`py` accepted)
  comemory search-code \"router\" --repo myrepo --lang rust

  # Caller-supplied vector (BYO-vector; code vectors are 768-dim)
  comemory search-code \"knn\" --vector 0.1,0.2,0.3,...

The working-set affinity boost applies only when search-code runs inside
the repo's working tree (the CWD is used to detect dirty/recent files).";

/// Arguments to `comemory search-code`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Natural-language or identifier query string.
    pub query: String,
    /// Override the configured `retrieval.top_k`. Must be >= 1.
    #[arg(
        long,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
    )]
    pub k: Option<usize>,
    /// Restrict hits to one repo label (as passed to `index-code --repo`).
    #[arg(long)]
    pub repo: Option<String>,
    /// Restrict hits to one language: `rust`, `typescript`, `javascript`,
    /// `python`, `go` (short aliases like `rs`/`ts`/`py` accepted).
    #[arg(long)]
    pub lang: Option<String>,
    /// Caller-supplied dense vector as a comma-separated float list.
    #[arg(long)]
    pub vector: Option<String>,
    /// Read a JSON `{ "embedding": [..] }` payload from stdin and use it as
    /// the dense vector for the query.
    #[arg(long, default_value_t = false)]
    pub vector_stdin: bool,
}

/// Run `comemory search-code`. Opens the DB, resolves the vector input
/// (if any), routes + reranks + cuts to `top_k`, records best-effort
/// telemetry (access bump + `retrieval_log` row, `source='search-code'`),
/// and emits results in either TTY or JSON form.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let lang = canonical_lang(a.lang.as_deref())?;
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;

    let vec = embedding_input::read_optional(a.vector_stdin, a.vector.as_deref())?;
    let cfg = override_top_k(load_config(&paths)?, a.k);
    let started = std::time::Instant::now();
    let candidates = code_route::route_code(
        &cfg,
        &conn,
        &a.query,
        vec.as_deref(),
        a.repo.as_deref(),
        lang,
    )?;
    let ws = cwd_working_set(a.repo.as_deref());
    let mut hits = code_rerank::rerank_code(&conn, &cfg, &candidates, &ws)?;
    hits.truncate(cfg.retrieval.top_k);
    let query_id = record_code_telemetry(
        &conn,
        &a.query,
        a.repo.as_deref(),
        lang,
        &hits,
        started.elapsed(),
    );
    // The empty-index probe only matters for the zero-hit TTY hint, so it
    // is skipped entirely whenever there are hits.
    let index_empty = hits.is_empty() && !code_index_populated(&conn)?;
    output::search_code::emit(&hits, query_id.as_deref(), index_empty, json_flag)
}

/// Validate and canonicalize the `--lang` flag through
/// [`Lang::parse`], so aliases (`rs`, `py`, ...) match the canonical
/// names stored in `code_symbols.lang` instead of silently filtering
/// everything out. The canonical form is also what lands in
/// `retrieval_log.kind`. Mirrors the gate in `cli::ast`.
fn canonical_lang(raw: Option<&str>) -> Result<Option<&'static str>> {
    match raw {
        None => Ok(None),
        Some(s) => Lang::parse(s).map(|l| Some(l.as_str())).ok_or_else(|| {
            Error::Config(format!(
                "unsupported --lang {:?}; supported: {}",
                s,
                languages::supported().join(", ")
            ))
        }),
    }
}

/// Build the working set from the process CWD (see the module doc for why
/// the CWD is the only checkout candidate available). The repo label is
/// the `--repo` filter when given, else the discovered working tree's
/// directory basename; with no resolvable label the affinity prior stays
/// neutral via the empty default set.
fn cwd_working_set(repo_filter: Option<&str>) -> WorkingSet {
    let Ok(cwd) = std::env::current_dir() else {
        return WorkingSet::default();
    };
    let label = match repo_filter {
        Some(r) => r.to_string(),
        None => match worktree_basename(&cwd) {
            Some(l) => l,
            None => return WorkingSet::default(),
        },
    };
    code_rerank::working_set(&cwd, &label)
}

/// Basename of the git working tree containing `cwd`, if any — the
/// default repo label for [`cwd_working_set`]. Bare repos (no workdir)
/// and non-repo paths return `None`.
fn worktree_basename(cwd: &Path) -> Option<String> {
    let repo = git2::Repository::discover(cwd).ok()?;
    let name = repo.workdir()?.file_name()?.to_str()?.to_string();
    Some(name)
}

/// `true` when at least one `code_symbols` row exists — distinguishes
/// "query missed" from "nothing was ever indexed" for the TTY hint.
fn code_index_populated(conn: &Connection) -> Result<bool> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM code_symbols)", [], |r| {
        r.get(0)
    })
    .map_err(Error::from)
}

/// Best-effort telemetry for one tracked code search, the code-side
/// sibling of `retrieval::pipeline::record_telemetry` (same one-tx /
/// fall-back-to-autocommit / never-fail contract, kept separate because
/// the access bump targets `code_symbols`, not `memories`): bump
/// `access_count`/`last_accessed` on the returned (parent) symbol ids and
/// insert the `retrieval_log` row in a single transaction. Returns the
/// logged query id, or `None` when logging failed.
fn record_code_telemetry(
    conn: &Connection,
    query: &str,
    repo: Option<&str>,
    lang: Option<&str>,
    hits: &[CodeReranked],
    elapsed: std::time::Duration,
) -> Option<String> {
    match conn.unchecked_transaction() {
        Ok(tx) => {
            record_code_access(&tx, hits);
            let query_id = record_code_query(&tx, query, repo, lang, hits, elapsed);
            match tx.commit() {
                Ok(()) => query_id,
                Err(e) => {
                    tracing::warn!(error = %e, "code telemetry commit failed; access counts and query log dropped");
                    None
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "code telemetry transaction unavailable; falling back to direct writes");
            record_code_access(conn, hits);
            record_code_query(conn, query, repo, lang, hits, elapsed)
        }
    }
}

/// Bump `code_symbols.access_count`/`last_accessed` for the returned
/// symbol ids (the coalesced parent ids — the feedback-able identities).
/// One `UPDATE ... WHERE id IN (...)` statement, timestamp via
/// [`memory_row::iso_format`] so every `last_accessed` writer emits the
/// same format. Best-effort: a failure must never break the read path.
fn record_code_access(conn: &Connection, hits: &[CodeReranked]) {
    if hits.is_empty() {
        return;
    }
    let now = match memory_row::iso_format(OffsetDateTime::now_utc()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "code access tracking skipped: timestamp format failed");
            return;
        }
    };
    let qmarks = crate::store::qmarks(hits.len());
    let sql = format!(
        "UPDATE code_symbols SET access_count = access_count + 1, last_accessed = ? \
         WHERE id IN ({qmarks})"
    );
    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(hits.len() + 1);
    params.push(&now);
    for h in hits {
        params.push(&h.symbol_id);
    }
    if let Err(e) = conn.execute(&sql, params.as_slice()) {
        tracing::warn!(error = %e, hit_count = hits.len(), "code access tracking update failed");
    }
}

/// Write the `retrieval_log` row for this code search: `source` is
/// `'search-code'`, the `repo`/`kind` columns carry the `--repo`/`--lang`
/// filters verbatim (`None` → NULL), and `returned_ids` is a JSON array
/// of symbol-id *strings* so the column shape matches the memory rows.
/// Best-effort like [`record_code_access`].
fn record_code_query(
    conn: &Connection,
    query: &str,
    repo: Option<&str>,
    lang: Option<&str>,
    hits: &[CodeReranked],
    elapsed: std::time::Duration,
) -> Option<String> {
    let now = OffsetDateTime::now_utc();
    let at = match memory_row::iso_format(now) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "code query logging skipped: timestamp format failed");
            return None;
        }
    };
    let query_id = crate::stats::feedback::generate_query_id(query, now);
    let ids: Vec<String> = hits.iter().map(|h| h.symbol_id.to_string()).collect();
    let returned = match serde_json::to_string(&ids) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "code query logging skipped: id serialization failed");
            return None;
        }
    };
    let dur = i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX);
    match conn.execute(
        "INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms,
                                   repo, kind, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'search-code')",
        rusqlite::params![query_id, query, returned, at, dur, repo, lang],
    ) {
        Ok(_) => Some(query_id),
        Err(e) => {
            tracing::warn!(error = %e, "code query logging failed");
            None
        }
    }
}
