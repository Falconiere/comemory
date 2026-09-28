//! [`Pass::push`], factored out of `replica_pass.rs` to keep that file under
//! the 300-code-line ceiling (see its own doc comment on the declaration).

use crate::domains::sync::drain::push::{self, Push};
use crate::domains::sync::drain::push_hold::{self, Context};
use crate::domains::sync::replica::contract::CursorRef;
use crate::prelude::*;
use crate::store::Connection;

use super::{Moved, Pass};

impl Pass<'_> {
    /// Re-decide holds, then send one batch; whether anything was sent.
    pub(super) fn push(&mut self, conn: &Connection) -> Result<Moved> {
        if !self.pushes {
            return Ok(Ok(false));
        }
        let (pull, cfg, at) = (self.step.pull, self.step.cfg, self.step.at);
        let skip = cfg.sync.skip_matcher()?;
        let capabilities = push_hold::capabilities(self.manifest);
        let upgrading = self
            .row
            .upgrade_through
            .filter(|through| self.cursor.applied_sequence < *through)
            .and(self.row.selected_at.as_deref());
        let ctx = Context {
            key: pull.key,
            policy: pull.policy,
            skip: &skip,
            capabilities: &capabilities,
            upgrading_since: upgrading,
        };
        self.report.held = u32::try_from(push_hold::classify(conn, &ctx, at)?).unwrap_or(u32::MAX);
        let cursor = (!self.cursor.stream_epoch.is_empty()).then(|| CursorRef {
            stream_epoch: self.cursor.stream_epoch.clone(),
            sequence: self.cursor.applied_sequence,
        });
        let push = Push {
            key: pull.key,
            transport: pull.transport,
            policy: pull.policy,
            cursor,
            max_bytes: usize::try_from(cfg.sync.max_request_bytes).unwrap_or(usize::MAX),
            epoch: Some(&self.manifest.stream_epoch),
            resting: self.unanswered.resting(),
        };
        let pushed = push::batch(conn, &push, at)?;
        self.unanswered.note(&pushed.unanswered);
        self.report.pushed += pushed.accepted;
        self.report.rejected += pushed.rejected;
        match pushed.failure {
            Some(failure) if failure.replaced_stream() => {
                self.rebootstrap(conn)?;
                Ok(Ok(true))
            }
            Some(failure) => Ok(Err(failure)),
            None => Ok(Ok(pushed.sent > 0)),
        }
    }
}
