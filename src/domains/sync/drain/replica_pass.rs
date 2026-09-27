//! One `replica-v1` pass: pull first, then alternate bounded pull pages and
//! push batches until the cursor reaches the head captured at session open
//! and nothing eligible is left, or the budget runs out.
//!
//! A replay in progress (a rebootstrap or a verify repair) resumes before any
//! forward pull, in the phase it stopped in. Every state change commits as it
//! happens — the key's row is saved after every iteration — so a pass killed
//! at any instant loses nothing.

use std::time::{Duration, Instant};

use crate::config::{Config, Paths};
use crate::domains::sync::drain::anchor::{self, Check};
use crate::domains::sync::drain::pull::{self, Pull, Step};
use crate::domains::sync::drain::push::{self, Push, Unanswered};
use crate::domains::sync::drain::push_hold::{self, Context};
use crate::domains::sync::drain::replay::{self, Replayed};
use crate::domains::sync::drain::report::{End, Report};
use crate::domains::sync::drain::session::{self, Legs, Mode, Session};
use crate::domains::sync::drain::transport::Failure;
use crate::domains::sync::drain::{adopt, code_capture, holds, network, rebootstrap, stop};
use crate::domains::sync::replica::contract::CursorRef;
use crate::domains::sync::replica::contract_views::ManifestResponse;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_cursor::{self, Cursor};
use crate::store::sync_exchange::{self, ExchangeRow};

/// A step's result: whether it moved anything, or the failure that ends the
/// pass.
type Moved = std::result::Result<bool, Failure>;

/// Run one replica pass and save the key's row.
///
/// # Errors
/// Configuration and SQLite failures; network failures end the pass with
/// [`End::Network`] and are recorded on the key.
pub fn run(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    session: &mut Session,
    (mode, legs): (Mode, Legs),
) -> Result<Report> {
    let at = network::now()?;
    let manifest = session
        .manifest
        .clone()
        .ok_or_else(|| Error::Other("a replica pass needs the manifest".into()))?;
    let Session {
        key,
        row,
        fingerprint,
        replica,
        policy,
        ..
    } = session;
    let pull = Pull {
        key,
        transport: replica,
        policy,
        managed_revision: replica.is_managed().then(|| policy.revision()),
    };
    let step = Step {
        paths,
        cfg,
        pull: &pull,
        at: &at,
    };
    let cursor = rebootstrap::cursor_of(conn, key)?;
    let mut pass = Pass::new(&step, &manifest, row, cursor, (mode, legs));
    let failure = match pass.prepare(conn)? {
        Some(failure) => Some(failure),
        None => pass.iterate(conn, session::budget(cfg, mode)?)?,
    };
    pass.finish(conn, failure.as_ref(), fingerprint)
}

/// The state one pass carries between its steps.
struct Pass<'a> {
    step: &'a Step<'a>,
    manifest: &'a ManifestResponse,
    row: &'a mut ExchangeRow,
    cursor: Cursor,
    pulls: bool,
    pushes: bool,
    /// How much of the event backlog this pass adopts before it pushes.
    reach: adopt::Reach,
    report: Report,
    /// The pull (or replay) stalled this pass; it is not retried until the
    /// next one.
    stalled: bool,
    /// The highest upstream head this pass saw (the manifest's, or a page's).
    seen_head: i64,
    /// Operations the upstream left unanswered this pass.
    unanswered: Unanswered,
}

impl<'a> Pass<'a> {
    /// A pass over `row` from `cursor`, on the stream `manifest` describes.
    fn new(
        step: &'a Step<'a>,
        manifest: &'a ManifestResponse,
        row: &'a mut ExchangeRow,
        cursor: Cursor,
        (mode, legs): (Mode, Legs),
    ) -> Self {
        Self {
            step,
            manifest,
            row,
            cursor,
            pulls: legs.pulls(mode),
            pushes: legs.pushes(),
            reach: adopt::Reach::of(mode),
            report: Report::default(),
            stalled: false,
            seen_head: manifest.head_sequence,
            unanswered: Unanswered::default(),
        }
    }

    /// Record how the pass ended on the key, save its row, and report.
    fn finish(
        self,
        conn: &Connection,
        failure: Option<&Failure>,
        fingerprint: &str,
    ) -> Result<Report> {
        let Self {
            row,
            mut report,
            seen_head,
            step,
            mut cursor,
            manifest,
            ..
        } = self;
        if failure.is_none() {
            row.upstream_head = Some(seen_head);
            // An upstream with nothing to read gives no page to name the
            // stream; the cursor still starts on it, at 0.
            if cursor.stream_epoch.is_empty() {
                cursor.stream_epoch.clone_from(&manifest.stream_epoch);
                replica_cursor::save(conn, &cursor, step.at)?;
            }
        }
        session::close(conn, row, (failure, fingerprint), &mut report, step.at)?;
        Ok(report)
    }

    /// Before the first batch: a replaced stream, event adoption, code
    /// capture, due holds, the anchor; `Some(failure)` when the anchor could not be read.
    fn prepare(&mut self, conn: &mut Connection) -> Result<Option<Failure>> {
        let replaced = !self.cursor.stream_epoch.is_empty()
            && (self.cursor.stream_epoch != self.manifest.stream_epoch
                || self.manifest.head_sequence < self.cursor.applied_sequence);
        if replaced {
            self.rebootstrap(conn)?;
        }
        let pull = self.step.pull;
        if self.pushes {
            let step = self.step;
            adopt::events((step.paths, step.cfg), conn, step.at, self.reach)?;
        }
        if self.pushes && self.pulls {
            code_capture::capture(conn, self.step.cfg, pull.policy)?;
        }
        if !self.pulls {
            return Ok(None);
        }
        holds::reconsider(conn, pull.key, pull.policy, &mut self.cursor, self.step.at)?;
        if replaced {
            return Ok(None);
        }
        match anchor::check(pull.transport, &self.cursor) {
            Ok(Check::Rewritten) => self.rebootstrap(conn)?,
            Ok(Check::Intact | Check::Unknown) => {}
            Err(failure) if failure.replaced_stream() => self.rebootstrap(conn)?,
            // The anchor read is a request like any other: its failure ends
            // the pass on the network rather than reading as intact.
            Err(failure) => return Ok(Some(failure)),
        }
        Ok(None)
    }

    /// Restart the cursor on the manifest's stream and begin a replay.
    fn rebootstrap(&mut self, conn: &Connection) -> Result<()> {
        let (epoch, head) = (&self.manifest.stream_epoch, self.manifest.head_sequence);
        rebootstrap::begin(conn, self.row, &mut self.cursor, epoch, head, self.step.at)?;
        sync_exchange::save(conn, self.row, self.step.at)?;
        self.report.rebootstrapped = true;
        Ok(())
    }

    /// Alternate pull and push until an end; `Some(failure)` ends it on the
    /// network.
    fn iterate(
        &mut self,
        conn: &mut Connection,
        budget: Option<Duration>,
    ) -> Result<Option<Failure>> {
        let started = Instant::now();
        loop {
            let pulled = match self.pull(conn)? {
                Ok(moved) => moved,
                Err(failure) => return Ok(Some(failure)),
            };
            let sent = match self.push(conn)? {
                Ok(sent) => sent,
                Err(failure) => return Ok(Some(failure)),
            };
            sync_exchange::save(conn, self.row, self.step.at)?;
            if let Some(end) = self.end(pulled, sent) {
                self.report.end = end;
                return Ok(None);
            }
            if let Some(end) = stop::boundary(self.step.paths, started, budget) {
                self.report.more = matches!(end, End::Budget | End::Cancelled);
                self.report.end = end;
                return Ok(None);
            }
        }
    }

    /// Why the pass ends after an iteration, if it does.
    fn end(&self, pulled: bool, sent: bool) -> Option<End> {
        if sent {
            return None;
        }
        if self.stalled {
            return Some(End::Stalled);
        }
        let caught_up = !self.pulls
            || (self.row.replay_state.is_none()
                && self.cursor.applied_sequence >= self.manifest.head_sequence);
        if caught_up {
            return Some(End::CaughtUp);
        }
        (!pulled).then_some(End::NoProgress)
    }

    /// One replay step or one forward pull page.
    fn pull(&mut self, conn: &mut Connection) -> Result<Moved> {
        if self.stalled || !self.pulls {
            return Ok(Ok(false));
        }
        if self.row.replay_state.is_some() {
            return self.replay(conn);
        }
        if self.cursor.applied_sequence >= self.manifest.head_sequence {
            return Ok(Ok(false));
        }
        let paged = pull::page(conn, self.step, &mut self.cursor)?;
        if paged.rebootstrap {
            self.rebootstrap(conn)?;
            return Ok(Ok(true));
        }
        if let Some(failure) = paged.failure {
            return Ok(Err(failure));
        }
        self.report.pulled += paged.applied;
        self.report.settled += paged.settled;
        self.report.more |= paged.head > self.manifest.head_sequence;
        self.seen_head = self.seen_head.max(paged.head);
        if let Some((sequence, reason)) = paged.stalled {
            self.stall(sequence, reason);
        } else if paged.advanced {
            self.row.stall_sequence = None;
            self.row.stall_reason = None;
        }
        if self
            .row
            .upgrade_through
            .is_some_and(|through| self.cursor.applied_sequence >= through)
        {
            self.row.upgrade_through = None;
        }
        Ok(Ok(paged.advanced || paged.settled > 0 || paged.held > 0))
    }

    /// Advance a replay in progress by one page.
    fn replay(&mut self, conn: &mut Connection) -> Result<Moved> {
        // A rebootstrap a killed or budgeted pass began is reported by the
        // pass that carries it on, as much as by the one that began it.
        self.report.rebootstrapped |=
            self.row.replay_kind.as_deref() == Some(rebootstrap::REBOOTSTRAP);
        match replay::step(conn, self.step, self.row, &mut self.cursor)? {
            Replayed::Moved(n) => self.report.pulled += n,
            Replayed::Stalled(sequence, reason) => {
                self.stall(sequence, reason);
                return Ok(Ok(false));
            }
            Replayed::Replaced => self.rebootstrap(conn)?,
            Replayed::Failed(failure) => return Ok(Err(failure)),
        }
        Ok(Ok(true))
    }

    /// Record a stall; the pass stops pulling until the next one.
    fn stall(&mut self, sequence: i64, reason: String) {
        self.row.stall_sequence = Some(sequence);
        self.row.stall_reason = Some(reason);
        self.stalled = true;
    }

    /// Re-decide holds, then send one batch; whether anything was sent.
    fn push(&mut self, conn: &Connection) -> Result<Moved> {
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
