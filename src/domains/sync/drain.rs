//! The `replica-v1` exchange client (#255): one pass drains this machine's
//! outbox upstream and the upstream's feed here, under the policy and protocol
//! the session negotiated, surviving every failure without losing or doubling
//! a change. Design: `docs/designs/2026-09-24-replica-exchange-client.md`.
//!
//! Folder index: [`drain/README.md`](drain/README.md).

/// Queueing local feed positions the outbox never saw.
pub mod adopt;
/// Whether the upstream still holds the entry the cursor was left on.
pub mod anchor;
/// Full-jitter delays between attempts.
pub mod backoff;
/// Capturing a code generation for upload, at push time.
pub mod code_capture;
/// Coming back for held pull positions once their cause is gone.
pub mod holds;
/// Whose data a pending operation is.
pub mod keying;
/// One legacy pass: the old protocol under the same loop.
pub mod legacy_pass;
/// Which protocol a key speaks.
pub mod negotiate;
/// The network state a key carries between passes.
pub mod network;
/// One pull page, resolved entry by entry.
pub mod pull;
/// What one pulled entry deserves (rules 1–7).
pub mod pull_rules;
/// One push batch and what the answer does to each row.
pub mod push;
/// Turning an outbox row into the operation a push sends; staged parts.
pub mod push_batch;
/// Why a pending operation is not sent this pass.
pub mod push_hold;
/// A replaced stream: restart the cursor and replay compactly.
pub mod rebootstrap;
/// One step of a replay in progress, in the phase it stopped in.
pub mod replay;
/// Phase 2 of a compacting replay: apply each entity's last entry.
pub mod replay_apply;
/// Phase 1 of a compacting replay: keep each entity's last entry.
pub mod replay_scan;
/// One `replica-v1` pass: alternate pull pages and push batches.
pub mod replica_pass;
/// What a pass did — the `exchange` leg of every sync report.
pub mod report;
/// `Retry-After` parsing.
pub mod retry_after;
/// Opening a pass: key, gates, policy, protocol.
pub mod session;
/// The `exchange` block of `sync --action status`.
pub mod status;
/// One request builder and one answer classification for upstream calls.
pub mod transport;
/// The first pass after a key selects `replica-v1`.
pub mod upgrade;
/// `sync --action verify` on a `replica-v1` key.
pub mod verify;

use crate::config::{Config, Paths};
use crate::domains::sync::AuthFile;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::sync_exchange::{self, ExchangeRow};
use legacy_pass::LegacyLegs;
use negotiate::Protocol;
use report::{End, Report};
use session::{Legs, Mode, Opened, Session};
use transport::Failure;

/// What a drain did.
#[derive(Debug, Default)]
pub struct Drained {
    /// The `exchange` leg: every pass of the run folded together.
    pub exchange: Report,
    /// Today's `pull` and `push` legs, when the key speaks the old protocol.
    pub legacy: Option<LegacyLegs>,
    /// Whether the origin is managed (the platform) — the only origin a legacy
    /// key also pushes the code index to.
    pub managed: bool,
    /// The key's recorded error, when the drain ended on the network.
    pub error: Option<String>,
}

/// Drain `auth`'s key: run passes until one ends without `more` (the inline
/// push runs exactly one). A policy that changes under a pass is reloaded and
/// the pass run once more at once; a second change backs off.
///
/// # Errors
/// Configuration, clock, store and markdown failures; network failures are
/// recorded on the key and reported as [`End::Network`].
pub fn drain(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    auth: &AuthFile,
    (mode, legs): (Mode, Legs),
) -> Result<Drained> {
    // A client that never saved anything has no memories directory yet, and
    // the first pulled memory is written there.
    paths.ensure_dirs()?;
    let mut drained = Drained::default();
    let mut reloaded = false;
    loop {
        let mut session = match session::open(conn, cfg, auth, mode)? {
            Opened::Ready(session) => session,
            Opened::Skipped(row) => {
                drained.exchange.absorb(skipped(&row));
                drained.error = row.last_error;
                return Ok(drained);
            }
        };
        drained.managed = session.replica.is_managed();
        let pass = run_pass(paths, cfg, conn, &mut session, (mode, legs), &mut drained)?;
        let changed = pass.policy_changed;
        drained.exchange.absorb(pass);
        if changed && !reloaded {
            reloaded = true;
            continue;
        }
        if changed {
            back_off_on_policy_churn(conn, &mut session)?;
            drained
                .exchange
                .network
                .clone_from(&session.row.network_state);
            drained.error = session.row.last_error;
            return Ok(drained);
        }
        if drained.exchange.end == End::Network {
            drained.error = session.row.last_error;
            return Ok(drained);
        }
        if !drained.exchange.more || matches!(mode, Mode::Inline(_)) {
            return Ok(drained);
        }
    }
}

/// One pass on whichever protocol the session selected, labelled with it.
fn run_pass(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    session: &mut Session,
    run: (Mode, Legs),
    drained: &mut Drained,
) -> Result<Report> {
    let mut pass = match session.protocol {
        Protocol::Replica => replica_pass::run(paths, cfg, conn, session, run)?,
        Protocol::Legacy => {
            let legs_run = drained.legacy.get_or_insert_default();
            legacy_pass::run(paths, cfg, conn, session, run, legs_run)?
        }
    };
    pass.protocol = Some(session.protocol.as_str().to_string());
    pass.coverage = Some(coverage(session.protocol).to_string());
    Ok(pass)
}

/// A policy that changed again after a reload backs off like an
/// unavailable upstream.
fn back_off_on_policy_churn(conn: &Connection, session: &mut Session) -> Result<()> {
    let failure = Failure::Unavailable(format!(
        "{}: the policy changed twice in one run",
        network::POLICY_CHANGED
    ));
    network::fail(&mut session.row, &failure, &session.fingerprint)?;
    sync_exchange::save(conn, &session.row, &network::now()?)
}

/// `full` on `replica-v1`; the old protocol carries memories and code only.
const fn coverage(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Replica => "full",
        Protocol::Legacy => "partial",
    }
}

/// The report of a pass that made no request.
fn skipped(row: &ExchangeRow) -> Report {
    let protocol = row
        .protocol
        .as_deref()
        .and_then(|p| Protocol::parse(Some(p)));
    Report {
        protocol: protocol.map(|p| p.as_str().to_string()),
        coverage: protocol.map(|p| coverage(p).to_string()),
        end: End::Network,
        network: row.network_state.clone(),
        ..Report::default()
    }
}

#[cfg(test)]
#[path = "drain/tests/drain.rs"]
mod tests;

/// Live-engine fixture shared by the drain module's tests.
#[cfg(test)]
#[path = "drain/tests/support.rs"]
pub(crate) mod test_support;
