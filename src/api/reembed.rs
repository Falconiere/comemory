//! `api::reembed` — `POST /api/v1/doctor/reembed`: re-vectorize memories
//! and/or code through the server's embed command (console-api spec §8).
//!
//! comemory is BYO-vector: nothing in the crate embeds. This is the one
//! place a vector is *derived* rather than supplied, and only because the
//! operator handed the server an embed command
//! (`--embed-cmd`/`COMEMORY_EMBED_CMD`) — the route answers `503
//! embedder_unavailable` when they did not, so [`run`] can take the command
//! as a plain `&str` and never has to reason about its absence.
//!
//! ## Failure model
//!
//! Three failure classes, deliberately handled differently:
//!
//! - **Dim mismatch** — the embedder returns a vector of the wrong width,
//!   so *every* row would fail identically. Hard error naming both dims
//!   (`Error::VecDimMismatch`), run aborts.
//! - **Embed command fails on the FIRST row** — the command is broken (bad
//!   path, no such model). Nothing has been written yet, so the whole run
//!   fails with `Error::Embedder` rather than reporting a "success" that
//!   re-embedded nothing.
//! - **Embed command fails on a LATER row** — a flaky embedder. The rows
//!   already re-embedded are durable (one transaction per row), so the run
//!   continues and the failure is counted in [`Response::failed`]. Losing
//!   thousands of good rows to one timeout is the worse outcome.
//!
//! Cancellation ([`ProgressSink::is_cancelled`]) is checked per row and
//! returns `Error::Cancelled`, which the job worker records as
//! `JobStatus::Cancelled`. Rows written before the cancel stay written —
//! there is no whole-run transaction to roll back, by design: progress must
//! be durable for a run that can take minutes.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::api::index_code::ProgressSink;
use crate::embed;
use crate::prelude::*;
use crate::store::{embed as store_embed, vector};

/// Emit one log line every this many processed rows — enough for a console
/// to see a long run moving without flooding the bounded 20-line tail.
const LOG_EVERY: u64 = 50;

/// Which side of the store `POST /doctor/reembed` re-vectorizes.
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    /// Live `memories` rows only.
    Memories,
    /// Parent `code_symbols` rows only.
    Code,
    /// Both, memories first (the default).
    #[default]
    Both,
}

impl Target {
    /// Whether this target includes the memory leg.
    fn memories(self) -> bool {
        matches!(self, Self::Memories | Self::Both)
    }

    /// Whether this target includes the code leg.
    fn code(self) -> bool {
        matches!(self, Self::Code | Self::Both)
    }
}

/// `POST /api/v1/doctor/reembed` request.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Which rows to re-vectorize; defaults to [`Target::Both`].
    #[serde(default)]
    pub target: Target,
    /// Accepted and currently ignored: the embed command is invoked once
    /// per row (`embed::embed_query` has no batch form), so there is no
    /// batch size to honor yet. Present so a client written to the draft
    /// spec is not rejected by `deny_unknown_fields`.
    #[serde(default)]
    pub batch: Option<usize>,
}

/// Row counts from one re-embed run.
#[derive(Serialize, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// `memory_vec` rows written.
    pub memories: u64,
    /// `code_vec` rows written.
    pub code: u64,
    /// Rows whose embed call failed after the first row succeeded.
    pub failed: u64,
    /// Rows with nothing to embed (empty body / empty snippet).
    pub skipped: u64,
}

/// Re-vectorize every row `req.target` selects through `embed_cmd`. See the
/// module doc for the failure and cancellation model. `sink`, when given,
/// receives per-row progress and a log line every [`LOG_EVERY`] rows.
pub fn run(
    ctx: &mut Ctx<'_>,
    req: Request,
    embed_cmd: &str,
    sink: Option<&dyn ProgressSink>,
) -> Result<Response> {
    let conn = ctx.conn()?;
    let memories = if req.target.memories() {
        memory_rows(conn)?
    } else {
        Vec::new()
    };
    let code = if req.target.code() {
        code_rows(conn)?
    } else {
        Vec::new()
    };
    let total = (memories.len() + code.len()) as u64;
    let mut state = RunState::new(embed_cmd, sink, total);
    reembed_memories(conn, &memories, &mut state)?;
    reembed_code(conn, &code, &mut state)?;
    if let Some(sink) = sink {
        sink.on_log(&format!("reembed: finished {total} row(s)"));
    }
    Ok(state.response)
}

/// Every live memory's `(id, body)`, ordered so a run is reproducible.
fn memory_rows(conn: &Connection) -> Result<Vec<(String, String)>> {
    let mut stmt =
        conn.prepare("SELECT id, body FROM memories WHERE deleted_at IS NULL ORDER BY id")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Every PARENT `code_symbols` row's `(id, snippet)`. Child chunk rows
/// (`parent_id IS NOT NULL`) carry no `code_vec` row of their own — the
/// parent's vector represents the symbol — so re-embedding them would write
/// rows the retrieval path never reads.
fn code_rows(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt =
        conn.prepare("SELECT id, snippet FROM code_symbols WHERE parent_id IS NULL ORDER BY id")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Re-embed every memory row, replacing its `memory_vec` row in its own
/// transaction so a mid-run failure or cancel leaves the rows already
/// written durable (module doc).
fn reembed_memories(
    conn: &mut Connection,
    rows: &[(String, String)],
    state: &mut RunState<'_>,
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let dim = vector::dim_memory(conn)?;
    for (id, body) in rows {
        state.guard_cancelled()?;
        if let Some(vec) = state.embed(body, dim)? {
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM memory_vec WHERE memory_id = ?1", [id])?;
            vector::insert_memory(&tx, id, &vec)?;
            tx.commit()?;
            state.response.memories += 1;
        }
        state.step();
    }
    Ok(())
}

/// Re-embed every parent code symbol, replacing its `code_vec` row in its
/// own transaction (see [`reembed_memories`]).
fn reembed_code(
    conn: &mut Connection,
    rows: &[(i64, String)],
    state: &mut RunState<'_>,
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let dim = vector::dim_code(conn)?;
    for (id, snippet) in rows {
        state.guard_cancelled()?;
        if let Some(vec) = state.embed(snippet, dim)? {
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM code_vec WHERE symbol_id = ?1", [id])?;
            vector::insert_code(&tx, *id, &vec)?;
            tx.commit()?;
            state.response.code += 1;
        }
        state.step();
    }
    Ok(())
}

/// Mutable run state shared by both legs: the embed command, the optional
/// progress sink, the running counters, and how many embed calls have been
/// attempted (the "first row" the module doc's failure model keys on).
struct RunState<'a> {
    cmd: &'a str,
    sink: Option<&'a dyn ProgressSink>,
    total: u64,
    done: u64,
    attempted: u64,
    response: Response,
}

impl<'a> RunState<'a> {
    /// A fresh state for a run of `total` rows.
    fn new(cmd: &'a str, sink: Option<&'a dyn ProgressSink>, total: u64) -> Self {
        Self {
            cmd,
            sink,
            total,
            done: 0,
            attempted: 0,
            response: Response::default(),
        }
    }

    /// `Err(Error::Cancelled)` when the caller asked this run to stop.
    fn guard_cancelled(&self) -> Result<()> {
        if self.sink.is_some_and(ProgressSink::is_cancelled) {
            return Err(Error::Cancelled);
        }
        Ok(())
    }

    /// Embed one row's `text`, applying the module doc's failure model:
    /// `Ok(None)` means "nothing written, keep going" (empty text, or a
    /// non-first-row embed failure already counted in `failed`).
    fn embed(&mut self, text: &str, dim: usize) -> Result<Option<Vec<f32>>> {
        if text.trim().is_empty() {
            self.response.skipped += 1;
            return Ok(None);
        }
        self.attempted += 1;
        match embed::embed_query(self.cmd, text) {
            Ok(vector) => {
                // A width mismatch is systematic, not per-row: fail the run
                // here rather than counting every row as `failed`.
                store_embed::guard_dim(&vector, dim)?;
                Ok(Some(vector))
            }
            Err(e) if self.attempted == 1 => Err(Error::Embedder(e.to_string())),
            Err(e) => {
                tracing::warn!(error = %e, "reembed: embed command failed for one row; continuing");
                self.response.failed += 1;
                Ok(None)
            }
        }
    }

    /// Record one processed row and report it through the sink.
    fn step(&mut self) {
        self.done += 1;
        let Some(sink) = self.sink else {
            return;
        };
        sink.on_progress(self.done, self.total);
        if self.done.is_multiple_of(LOG_EVERY) {
            sink.on_log(&format!("reembed: {}/{} rows", self.done, self.total));
        }
    }
}

#[cfg(test)]
#[path = "tests/reembed.rs"]
mod tests;
