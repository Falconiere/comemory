//! Read-only interactive terminal explorer over the memory + code index.
//!
//! `comemory tui` drives the existing retrieval pipeline verbatim — lexical
//! live search (FTS5, no embedder) with optional Memory-tab vector enrichment
//! — behind a ratatui front end. A dedicated blocking DB-worker owns the single
//! SQLite connection; the async render loop talks to it over channels and never
//! touches the store directly. Nothing here mutates the index.
//!
//! The orchestrator spawns the DB-worker, drives an async `EventStream` +
//! `tokio::select!` loop (debounced lexical search, on-demand semantic enrich,
//! stale-response discard by `seq`), and on Enter prints the selected id to
//! stdout after the terminal is restored.

use std::io::Write;
use std::path::PathBuf;

use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use tokio::time::Instant;

use crate::config::paths::Paths;
use crate::prelude::*;
use crate::store::connection;
use crate::tui::app::{App, Effect};
use crate::tui::search::DEBOUNCE;
use crate::tui::terminal::TerminalGuard;
use crate::tui::view::layout;
use crate::tui::worker::Request;

/// Pure UI state (`App`, `Action`, `Effect`) and its transitions.
pub mod app;
/// Embed-command shell-out for Memory-tab semantic enrichment.
pub mod embed;
/// Pure key-press → `Action` mapping.
pub mod event;
/// Preview text for the selected row (pure formatting).
pub mod preview;
/// Render-side bridge: build requests, apply responses, discard stale ones.
pub mod search;
/// RAII terminal lifecycle (raw mode + alternate screen on stderr).
pub mod terminal;
/// ratatui widgets (pure render from `&App`).
pub mod view;
/// The DB-worker: owns the connection and serves search requests.
pub mod worker;

/// What the event loop should do after handling an input event.
enum Flow {
    /// Keep looping (redraw).
    Continue,
    /// Exit, optionally printing the selected id.
    Exit(Option<String>),
}

/// Launch the explorer. Resolves the data dir, opens the index, spawns the
/// DB-worker, runs the interactive loop, restores the terminal, then prints any
/// Enter-selected id to stdout. Returns once the user quits.
pub async fn run(
    repo: Option<String>,
    query: Option<String>,
    embed_cmd: Option<String>,
    data_dir: Option<PathBuf>,
) -> Result<()> {
    let paths = Paths::new(crate::cli::resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;
    let cfg = crate::cli::load_config(&paths)?;
    let page_size = cfg.retrieval.top_k;

    let (req_tx, req_rx) = std::sync::mpsc::channel::<Request>();
    let (resp_tx, mut resp_rx) = tokio::sync::mpsc::unbounded_channel::<worker::Response>();
    let handle = worker::spawn(cfg, conn, req_rx, resp_tx);

    let mut app = App::new(repo, query, page_size);
    app.has_embedder = embed_cmd.is_some();
    let selection = {
        let mut guard = TerminalGuard::enter()?;
        run_loop(
            &mut guard,
            &mut app,
            &req_tx,
            &mut resp_rx,
            embed_cmd.as_deref(),
        )
        .await?
    }; // terminal restored here, before any stdout write

    drop(req_tx); // closing the request channel stops the worker
    let _ = handle.join();

    if let Some(id) = selection {
        let mut out = std::io::stdout();
        let _ = writeln!(out, "{id}");
    }
    Ok(())
}

/// The async render/event loop. Returns the Enter-selected id, or `None` on
/// quit.
async fn run_loop(
    guard: &mut TerminalGuard,
    app: &mut App,
    req_tx: &std::sync::mpsc::Sender<Request>,
    resp_rx: &mut tokio::sync::mpsc::UnboundedReceiver<worker::Response>,
    embed_cmd: Option<&str>,
) -> Result<Option<String>> {
    let mut events = EventStream::new();
    let mut deadline: Option<Instant> = None;
    dispatch(app, req_tx, None, false); // initial lexical search of the seed query
    loop {
        let _ = guard
            .terminal()
            .draw(|f| layout::render(f, app))
            .map_err(Error::Io)?;
        tokio::select! {
            ev = events.next() => match handle_event(app, ev, req_tx, embed_cmd, &mut deadline) {
                Flow::Continue => {}
                Flow::Exit(sel) => return Ok(sel),
            },
            Some(resp) = resp_rx.recv() => search::apply_response(app, resp),
            _ = wait_until(deadline) => {
                dispatch(app, req_tx, None, false);
                deadline = None;
            }
        }
    }
}

/// Decode one terminal event, apply it to the app, and report the next [`Flow`].
/// A closed/errored input stream quits cleanly.
fn handle_event(
    app: &mut App,
    ev: Option<std::result::Result<Event, std::io::Error>>,
    req_tx: &std::sync::mpsc::Sender<Request>,
    embed_cmd: Option<&str>,
    deadline: &mut Option<Instant>,
) -> Flow {
    let key = match ev {
        Some(Ok(Event::Key(k))) if k.kind != KeyEventKind::Release => k,
        Some(Ok(_)) => return Flow::Continue,
        Some(Err(_)) | None => return Flow::Exit(None),
    };
    match app.apply(event::map_key(key)) {
        Effect::Redraw => Flow::Continue,
        Effect::Search => {
            *deadline = Some(Instant::now() + DEBOUNCE);
            Flow::Continue
        }
        Effect::Semantic => {
            // App only returns Semantic when an embedder is configured
            // (`App::has_embedder`), so a dispatch here always has a command.
            // Clear any pending debounce: the semantic request dispatches now at
            // the current seq, and a still-armed deadline would otherwise fire a
            // *lexical* request at the SAME seq — both responses would pass
            // `is_current` and race, making `enriched`/`memory_hits`
            // arrival-order-dependent.
            *deadline = None;
            dispatch(app, req_tx, embed_cmd, true);
            Flow::Continue
        }
        Effect::Accept => Flow::Exit(app.selected_id()),
        Effect::Quit => Flow::Exit(None),
    }
}

/// Send a search request for the app's current state to the DB-worker.
fn dispatch(
    app: &App,
    req_tx: &std::sync::mpsc::Sender<Request>,
    embed_cmd: Option<&str>,
    semantic: bool,
) {
    let _ = req_tx.send(search::build_request(app, embed_cmd, semantic));
}

/// Sleep until `deadline`, or never when it is `None` (no pending search).
async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending::<()>().await,
    }
}
