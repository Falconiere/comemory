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
//! process CWD is used as the working-tree candidate (via the shared
//! [`WorkingSet::from_cwd`] policy, which also covers `context`), so the
//! affinity boost activates only when the command runs inside the
//! relevant repo's checkout (documented in `--help`). The detection is
//! best-effort by contract — a non-repo CWD degrades to the empty set
//! and a neutral prior.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use rusqlite::Connection;

use crate::ast::languages::{self, Lang};
use crate::cli::{
    embedding_input, lazy_reindex, load_config, page_meta, page_window, resolve_data_dir,
};
use crate::config::paths::Paths;
use crate::output;
use crate::prelude::*;
use crate::retrieval::code_rerank::{self, CodeReranked, WorkingSet};
use crate::retrieval::{code_route, pipeline};
use crate::store::{code_row, connection};

// The closing working-set caveat paragraph is intentionally duplicated in
// `cli::context::EXAMPLES` (same semantics; only the command name and the
// indexed/referenced adjective differ). clap's `after_help` plus the
// regenerated docs/cli-reference.md freeze the exact wrapped text, so a
// shared const cannot reproduce both renderings. A drift tripwire in
// `tests/cli/search_code.rs` asserts the two paragraphs stay equivalent.
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
the indexed repo's checkout (the CWD is used to detect dirty/recent files)
AND the repo label used at index time (`index-code --repo`) matches the
--repo flag — or, when --repo is omitted, the checkout directory's
basename.";

/// Arguments to `comemory search-code`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Natural-language or identifier query string.
    pub query: String,
    /// Page size — overrides the configured `retrieval.top_k`. `--limit`
    /// is an accepted alias. `0` means "all remaining within the
    /// `max_page_window`".
    #[arg(long, visible_alias = "limit")]
    pub k: Option<usize>,
    /// Number of leading ranked results to skip (deep paging). Bounded by
    /// `retrieval.max_page_window`; once the window ceiling is reached
    /// `has_more` is false and deeper results require refining the query.
    #[arg(long, default_value_t = 0)]
    pub offset: usize,
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
    let cfg = load_config(&paths)?;
    // Non-blocking lazy auto-reindex: under `auto_reindex = lazy`, fire a
    // detached `index-code` when this repo's HEAD has moved since the last
    // index, then search the current (possibly slightly stale) index. No-op
    // for `hook`/`off`, off-repo, or a fresh index. Never blocks or fails the
    // search (see `cli::lazy_reindex`).
    lazy_reindex::maybe_trigger(&conn, &cfg, &paths, a.repo.as_deref());
    let window = page_window(&cfg, a.k, a.offset);
    let max_window = cfg.retrieval.max_page_window;
    let pool = pipeline::pool_size(window.offset, window.limit, max_window);
    let started = std::time::Instant::now();
    let candidates = code_route::route_code(
        &cfg,
        &conn,
        &a.query,
        vec.as_deref(),
        a.repo.as_deref(),
        lang,
        pool,
    )?;
    // Zero candidates → nothing for the affinity prior to boost, so the
    // git discovery + status walk behind `WorkingSet::from_cwd` is skipped.
    let ws = if candidates.is_empty() {
        WorkingSet::default()
    } else {
        WorkingSet::from_cwd(a.repo.as_deref())
    };
    // Rerank produces the full ranked (post-coalesce) window; the page is
    // sliced from it via the shared paginator so search-code, search, and
    // context agree on window semantics.
    let ranked = code_rerank::rerank_code(&conn, &cfg, &candidates, &ws)?;
    let (hits, has_more, total) = pipeline::paginate(ranked, window, max_window);
    // Telemetry reflects the RETURNED page only (consistent with `search`):
    // the access bump + retrieval_log row cover the sliced ids.
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
    let meta = page_meta(window, has_more, total);
    output::search_code::emit(&hits, query_id.as_deref(), meta, index_empty, json_flag)
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
/// fall-back-to-autocommit / never-fail contract; the tx scaffold is
/// deliberately a thin local twin rather than a shared closure-taking
/// helper — the two access-bump + log sequences differ in target table
/// and id mapping, and a generic scaffold would contort both callers):
/// bump `access_count`/`last_accessed` on the returned (parent) symbol
/// ids and insert the `retrieval_log` row in a single transaction via the
/// shared [`pipeline::log_retrieval`] writer, with the symbol ids
/// text-encoded so the `returned_ids` column shape matches the memory
/// rows and the `repo`/`kind` columns carrying the `--repo`/`--lang`
/// filters verbatim. Returns the logged query id, or `None` when logging
/// failed.
fn record_code_telemetry(
    conn: &Connection,
    query: &str,
    repo: Option<&str>,
    lang: Option<&str>,
    hits: &[CodeReranked],
    elapsed: std::time::Duration,
) -> Option<String> {
    let ids: Vec<String> = hits.iter().map(|h| h.symbol_id.to_string()).collect();
    let log = |conn: &Connection| {
        pipeline::log_retrieval(
            conn,
            query,
            &ids,
            elapsed,
            repo,
            lang,
            crate::stats::source::SEARCH_CODE,
        )
    };
    match conn.unchecked_transaction() {
        Ok(tx) => {
            record_code_access(&tx, hits);
            let query_id = log(&tx);
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
            log(conn)
        }
    }
}

/// Bump `code_symbols.access_count`/`last_accessed` for the returned
/// symbol ids (the coalesced parent ids — the feedback-able identities)
/// via the shared [`code_row::record_access`] writer, so `search-code`
/// and `context` cannot drift on the bump SQL or its best-effort
/// contract.
fn record_code_access(conn: &Connection, hits: &[CodeReranked]) {
    let ids: Vec<i64> = hits.iter().map(|h| h.symbol_id).collect();
    code_row::record_access(conn, &ids);
}
