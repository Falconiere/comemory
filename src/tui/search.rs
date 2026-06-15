//! Render-side bridge between [`App`] state and the DB-worker.
//!
//! Turns the app's current query/tab/window into a [`Request`] (lexical or
//! Memory-tab semantic), and applies worker [`Response`]s back onto the app —
//! discarding any whose generation `seq` is stale (a superseded query). The
//! debounce that coalesces keystrokes lives in the event loop; the interval is
//! published here as [`DEBOUNCE`].

use std::time::Duration;

use crate::tui::app::App;
use crate::tui::worker::{Hits, Request, Response};

/// Debounce window for as-you-type searches: keystrokes within this interval
/// coalesce into a single dispatched query. Tunable.
pub const DEBOUNCE: Duration = Duration::from_millis(90);

/// Build the DB-worker request for the app's current state. When `semantic` is
/// set (Memory-tab Ctrl-S with a configured embedder), the request carries the
/// embed command so the worker vectorizes the query; otherwise it is lexical.
pub fn build_request(app: &App, embed_cmd: Option<&str>, semantic: bool) -> Request {
    Request {
        seq: app.seq,
        query: app.query.clone(),
        vec: None,
        repo: app.repo.clone(),
        tab: app.tab,
        window: app.window,
        embed: if semantic {
            embed_cmd.map(str::to_string)
        } else {
            None
        },
    }
}

/// Whether a response is current — its `seq` matches the app's generation.
/// Stale responses (from a superseded query) must be discarded.
pub fn is_current(app: &App, resp: &Response) -> bool {
    resp.seq == app.seq
}

/// Apply a response to the app when current: set the active tab's hits, mark
/// `enriched` for a semantic Memory result, or surface an error on the status
/// line. Stale responses are ignored.
pub fn apply_response(app: &mut App, resp: Response) {
    if !is_current(app, &resp) {
        return;
    }
    match resp.result {
        Ok(Hits::Memory { hits, has_more }) => {
            app.enriched = resp.semantic;
            app.set_memory_hits(hits, has_more);
        }
        Ok(Hits::Code { hits, has_more }) => app.set_code_hits(hits, has_more),
        Err(msg) => app.status = msg,
    }
}
