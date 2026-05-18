//! Shared HTTP server state. The kuzu [`Connection`] is `!Sync`, so all
//! handlers serialise behind a `Mutex` around the long-lived [`Graph`].

use std::sync::{Arc, Mutex};

use crate::config::paths::Paths;
use crate::graph::Graph;

/// Long-lived state injected into every axum handler.
#[derive(Clone)]
pub struct ServerState {
    /// Shared kuzu graph handle. Cloning is cheap (`Arc`).
    pub graph: Arc<Mutex<Graph>>,
    /// Resolved data-dir layout. Used by handlers that need to load memory
    /// markdown bodies from disk.
    pub paths: Arc<Paths>,
}

impl ServerState {
    /// Build a new state by taking ownership of the [`Graph`] and the
    /// [`Paths`].
    pub fn new(graph: Graph, paths: Paths) -> Self {
        Self {
            graph: Arc::new(Mutex::new(graph)),
            paths: Arc::new(paths),
        }
    }
}
