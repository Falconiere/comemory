//! [`Pass::seed_local`], factored out of `replica_pass.rs` to keep that file
//! under the 300-code-line ceiling (see its own doc comment on the
//! declaration).
//!
//! `replica::bootstrap`, `seed_trash` and `seed_documents` are the replica
//! routes' own capability-gating walks — a hub advances them when a peer
//! asks for its manifest or changes. A client is never asked, so nothing
//! else ever advances its own pre-journal memories, trash or documents;
//! this is what does, once per pass, before the events `adopt` captures and
//! before anything is pushed.

use crate::domains::sync::replica::{bootstrap, seed_documents, seed_trash};
use crate::prelude::*;
use crate::store::Connection;
use crate::utilities::context::Ctx;

use super::Pass;

/// Batches each walk may take in one pass — bounded so a very large
/// pre-journal backlog still returns control to the pass loop instead of
/// blocking a `comemory sync` indefinitely; the rest resumes next pass.
const MAX_BATCHES: usize = 50;

impl Pass<'_> {
    /// Advance this engine's own pre-journal memory, trash and document
    /// seeding, each up to [`MAX_BATCHES`] batches.
    pub(super) fn seed_local(&self, conn: &mut Connection) -> Result<()> {
        let mut ctx = Ctx::borrowed(self.step.paths, self.step.cfg, conn);
        for _ in 0..MAX_BATCHES {
            if bootstrap::advance(&mut ctx)?.complete() {
                break;
            }
        }
        for _ in 0..MAX_BATCHES {
            if seed_trash::advance(&mut ctx)?.complete() {
                break;
            }
        }
        for _ in 0..MAX_BATCHES {
            if seed_documents::advance(&mut ctx)?.complete() {
                break;
            }
        }
        Ok(())
    }
}
