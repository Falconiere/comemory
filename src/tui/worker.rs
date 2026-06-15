//! The DB-worker: owns the single SQLite connection and serves search requests.
//!
//! `store::connection::open` hands back one plain `rusqlite::Connection`
//! (`Send`, `!Sync`) — it cannot be shared by reference with the async render
//! loop. So a dedicated `std::thread` owns it and answers [`Request`]s over a
//! channel, returning a [`Response`] tagged with the request's generation
//! `seq`. Every query is **read-only**: the memory leg sets `track = false`,
//! the code leg skips telemetry, so no row is mutated.

use rusqlite::Connection;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::code_rerank::{self, CodeReranked, WorkingSet};
use crate::retrieval::code_route;
use crate::retrieval::pipeline::{self, PageWindow, SearchOptions};
use crate::retrieval::rerank::Reranked;
use crate::tui::app::Tab;
use crate::tui::embed;

/// A search request tagged with the generation counter that produced it.
pub struct Request {
    /// Generation counter; echoed back so the loop can discard stale results.
    pub seq: u64,
    /// The query string.
    pub query: String,
    /// Optional dense query vector (Memory-tab semantic enrich only).
    pub vec: Option<Vec<f32>>,
    /// Optional repo filter forwarded to the leg.
    pub repo: Option<String>,
    /// Which index to search.
    pub tab: Tab,
    /// The `(offset, limit)` page of the bounded ranked window.
    pub window: PageWindow,
    /// When set (Memory-tab semantic enrich), the worker shells out to this
    /// embed command to vectorize `query` before searching. `None` → lexical.
    pub embed: Option<String>,
}

/// The page of hits for one tab.
pub enum Hits {
    /// Memory hits from `pipeline::search`.
    Memory {
        /// The ranked page.
        hits: Vec<Reranked>,
        /// Whether more in-window results exist beyond this page.
        has_more: bool,
    },
    /// Code hits from `code_rerank::rerank_code`.
    Code {
        /// The ranked page.
        hits: Vec<CodeReranked>,
        /// Whether more in-window results exist beyond this page.
        has_more: bool,
    },
}

/// A worker response carrying the request's `seq`. `result` is `Err(message)`
/// when the query failed — the render loop surfaces it on the status line and
/// keeps the prior page; the error is converted, never swallowed.
pub struct Response {
    /// The originating request's generation counter.
    pub seq: u64,
    /// Whether the request was a semantic (embedded) Memory query, so the
    /// loop can set `App::enriched` only when a vector was actually used.
    pub semantic: bool,
    /// The page of hits, or a human-readable error message.
    pub result: std::result::Result<Hits, String>,
}

/// Run one request against the index. Read-only.
pub fn run_query(cfg: &Config, conn: &Connection, req: &Request) -> Result<Hits> {
    match req.tab {
        Tab::Memory => run_memory(cfg, conn, req),
        Tab::Code => run_code(cfg, conn, req),
    }
}

/// Memory leg: drive `pipeline::search` with telemetry tracking off. When the
/// request carries an embed command, vectorize the query first (semantic
/// enrich); otherwise use any caller-supplied vector, else go lexical.
fn run_memory(cfg: &Config, conn: &Connection, req: &Request) -> Result<Hits> {
    let owned = match &req.embed {
        Some(cmd) => Some(embed::embed_query(cmd, &req.query)?),
        None => req.vec.clone(),
    };
    let run = pipeline::search(
        cfg,
        conn,
        &req.query,
        owned.as_deref(),
        req.repo.as_deref(),
        None,
        SearchOptions {
            track: false,
            source: crate::stats::source::SEARCH,
            window: req.window,
        },
    )?;
    Ok(Hits::Memory {
        hits: run.hits,
        has_more: run.has_more,
    })
}

/// Code leg: the `search-code` stages (route → working-set → rerank →
/// paginate) without the CLI's lazy-reindex trigger or telemetry write.
fn run_code(cfg: &Config, conn: &Connection, req: &Request) -> Result<Hits> {
    let max_window = cfg.retrieval.max_page_window;
    let pool = pipeline::pool_size(req.window.offset, req.window.limit, max_window);
    let candidates = code_route::route_code(
        cfg,
        conn,
        &req.query,
        req.vec.as_deref(),
        req.repo.as_deref(),
        None,
        pool,
    )?;
    let ws = if candidates.is_empty() {
        WorkingSet::default()
    } else {
        WorkingSet::from_cwd(req.repo.as_deref())
    };
    let ranked = code_rerank::rerank_code(conn, cfg, &candidates, &ws)?;
    let (hits, has_more, _total) = pipeline::paginate(ranked, req.window, max_window);
    Ok(Hits::Code { hits, has_more })
}

/// Spawn the DB-worker thread. It owns `cfg` + `conn`, serves `Request`s from
/// `rx` until the channel closes, and emits a `Response` per request on `tx`.
/// A query error is converted to a message and carried in the response (never
/// swallowed, never a panic), so the render loop stays responsive.
pub fn spawn(
    cfg: Config,
    conn: Connection,
    rx: std::sync::mpsc::Receiver<Request>,
    tx: tokio::sync::mpsc::UnboundedSender<Response>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(req) = rx.recv() {
            let semantic = req.embed.is_some();
            let result = run_query(&cfg, &conn, &req).map_err(|e| e.to_string());
            let resp = Response {
                seq: req.seq,
                semantic,
                result,
            };
            if tx.send(resp).is_err() {
                break; // The loop dropped its receiver; nothing left to serve.
            }
        }
    })
}
