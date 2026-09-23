//! What this machine knows about a repo it has no checkout of.
//!
//! A pulled generation is a second, parallel body of knowledge: it never
//! writes a local row, so nothing that reads `code_symbols` can see it. Two
//! readers need to see both sides anyway — the repository inventory, which
//! must list a repo a peer shared, and the code graph, which must answer for
//! it. This module states that union once, so neither reader invents its own
//! rule for which side wins.
//!
//! Source search is deliberately NOT unioned: a pulled generation carries no
//! snippet by construction, so there is nothing for `search-code` to return
//! and nothing to rank. That asymmetry is the design, not a gap.

use crate::prelude::*;
use crate::store::Connection;
use crate::store::edges::GraphEdgeRow;
use crate::store::remote_code_view::{self, SharedRepo};

/// Every repo whose current state came from a peer rather than a checkout
/// here, ascending by label.
///
/// # Errors
/// Propagates SQLite failures.
pub fn repos(conn: &Connection) -> Result<Vec<SharedRepo>> {
    remote_code_view::shared_repos(conn)
}

/// The edges the shared side contributes to the code graph, optionally scoped
/// to one repo.
///
/// A file pair the local index already states is not repeated here: the graph
/// shows one edge carrying the local weight. The paginated graph query reads
/// the same definition, so this answer and a windowed page agree about what
/// the shared side holds.
///
/// # Errors
/// Propagates SQLite failures.
pub fn graph_edges(conn: &Connection, repo: Option<&str>) -> Result<Vec<GraphEdgeRow>> {
    remote_code_view::shared_edges(conn, repo)
}

#[cfg(test)]
#[path = "tests/remote_view.rs"]
mod tests;
