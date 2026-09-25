//! One legacy pass: the old protocol's pull pages and push batches under the
//! same loop, budget and network state as a `replica-v1` pass.
//!
//! The legacy wire keeps its shapes. What the drain adds is the unmanaged
//! origin rule (no managed headers, no `repositories` map), a pull that stops
//! before an entry it did not apply, and pages until caught up instead of a
//! 2,000-entry cap.

use std::time::{Duration, Instant};

use crate::config::{Config, Paths};
use crate::domains::sync::drain::report::{End, Report};
use crate::domains::sync::drain::session::{self, Legs, Mode, Session};
use crate::domains::sync::drain::transport::Failure;
use crate::domains::sync::drain::{network, stop};
use crate::domains::sync::pull::{self, PullStats};
use crate::domains::sync::push::{self, PushStats, Wire};
use crate::prelude::*;
use crate::store::sync_exchange::ExchangeRow;
use crate::store::{Connection, sync_log};

/// The legacy legs of a run, reported as today's `pull` and `push` objects.
#[derive(Debug, Clone, Default)]
pub struct LegacyLegs {
    /// The pull, when the run pulled.
    pub pull: Option<PullStats>,
    /// The memory push, when the run pushed.
    pub push: Option<PushStats>,
}

/// Run one legacy pass, folding its counters into `legs_run`.
///
/// # Errors
/// Configuration, store and markdown failures, and a push the upstream
/// refused under repository policy; network failures end the pass with
/// [`End::Network`] and are recorded on the key.
pub fn run(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    session: &mut Session,
    (mode, legs): (Mode, Legs),
    legs_run: &mut LegacyLegs,
) -> Result<Report> {
    let at = network::now()?;
    let skip = cfg.sync.skip_matcher()?;
    let Session {
        key,
        row,
        fingerprint,
        legacy,
        policy,
        ..
    } = session;
    if legs.pulls(mode) {
        legs_run.pull.get_or_insert_default().stalled = None;
    }
    if legs.pushes() {
        legs_run.push.get_or_insert_default();
        sync_log::backfill_missing_local(conn)?;
    }
    let before = legs_run.clone();
    let pass = Pass {
        paths,
        cfg,
        wire: Wire {
            transport: legacy,
            key,
            policy,
            skip: &skip,
        },
        at: &at,
    };
    let mut report = Report::default();
    let failure = pass.iterate(
        conn,
        (legs.pulls(mode), legs.pushes()),
        legs_run,
        &mut report,
        session::budget(cfg, mode)?,
    )?;
    summarize((&before, legs_run), row, &mut report);
    session::close(conn, row, (failure.as_ref(), fingerprint), &mut report, &at)?;
    Ok(report)
}

/// What every step of one pass reads.
struct Pass<'a> {
    paths: &'a Paths,
    cfg: &'a Config,
    wire: Wire<'a>,
    at: &'a str,
}

impl Pass<'_> {
    /// Alternate pull pages and push batches until both are done, nothing
    /// moves or the budget runs out; `Some(failure)` ends it on the network.
    fn iterate(
        &self,
        conn: &mut Connection,
        (mut pulling, mut pushing): (bool, bool),
        legs: &mut LegacyLegs,
        report: &mut Report,
        budget: Option<Duration>,
    ) -> Result<Option<Failure>> {
        let started = Instant::now();
        loop {
            let mut moved = false;
            if let Some(stats) = legs.pull.as_mut().filter(|_| pulling) {
                match pull::page(
                    self.paths,
                    self.cfg,
                    conn,
                    (self.wire.transport, self.wire.key),
                    stats,
                    self.at,
                )? {
                    Ok(paged) => (moved, pulling) = (paged.advanced, !paged.done),
                    Err(failure) => return Ok(Some(failure)),
                }
            }
            if let Some(stats) = legs.push.as_mut().filter(|_| pushing) {
                match push::batch(self.paths, conn, &self.wire, stats)? {
                    Ok(sent) => (moved, pushing) = (moved || sent, sent),
                    Err(failure) => return Ok(Some(failure)),
                }
            }
            if !pulling && !pushing {
                return Ok(None);
            }
            if !moved {
                report.end = End::NoProgress;
                return Ok(None);
            }
            if let Some(end) = stop::boundary(self.paths, started, budget) {
                report.more = end == End::Budget;
                report.end = end;
                return Ok(None);
            }
        }
    }
}

/// This pass's `exchange` counters (the legs are the run's totals) and the
/// key's stall state.
fn summarize(
    (before, legs): (&LegacyLegs, &LegacyLegs),
    row: &mut ExchangeRow,
    report: &mut Report,
) {
    if let Some(pull) = &legs.pull {
        let earlier = before.pull.as_ref().map_or((0, 0), |p| (p.pulled, p.held));
        report.pulled = pull.pulled - earlier.0;
        report.held = pull.held - earlier.1;
        row.stall_sequence = pull.stalled.as_ref().map(|(sequence, _)| *sequence);
        row.stall_reason = pull.stalled.as_ref().map(|(_, reason)| reason.clone());
        if pull.stalled.is_some() {
            report.end = End::Stalled;
        }
    }
    if let Some(push) = &legs.push {
        let earlier = before.push.clone().unwrap_or_default();
        report.pushed = push.pushed - earlier.pushed;
        report.rejected = push.rejected_repo - earlier.rejected_repo;
        let withheld = |p: &PushStats| p.skipped_config + p.blocked_repo + p.blocked_secrets;
        report.held += withheld(push) - withheld(&earlier);
    }
}
