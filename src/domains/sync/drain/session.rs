//! Opening a pass: which key, whether the network may be used, which policy
//! and which protocol — or why this pass makes no request at all.
//!
//! Order matters. A suspended credential and a backoff are checked before any
//! request; the policy is loaded before the manifest because a managed origin
//! must be asked under the revision it will echo; the manifest is read on
//! every pass because it carries the captured head, the stream epoch and the
//! kinds the upstream can read.

use std::time::Duration;

use crate::config::Config;
use crate::config::sync::parse_duration;
use crate::domains::sync::AuthFile;
use crate::domains::sync::client_policy::SyncPolicyStatus;
use crate::domains::sync::client_protocol::SYNC_PROTOCOL;
use crate::domains::sync::drain::negotiate::{self, Decision, Manifest, Protocol};
use crate::domains::sync::drain::report::{End, Report};
use crate::domains::sync::drain::transport::{Failure, Transport};
use crate::domains::sync::drain::{keying, network, upgrade};
use crate::domains::sync::replica::contract::PROTOCOL as REPLICA;
use crate::domains::sync::replica::contract_views::ManifestResponse;
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::sync_exchange::{self, ExchangeKey, ExchangeRow};
use crate::utilities::digest::sha256_hex;

/// Who runs the pass, which decides the timeouts and the backoff gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A person ran `comemory sync`: only an upstream-asked backoff is honored.
    Manual,
    /// A hook, the daemon or a watch nudge: every backoff is honored.
    Unattended,
    /// The push right after a save, under its own small timeout.
    Inline(Duration),
}

/// Which directions a pass exchanges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Legs {
    /// Pull, then push (`comemory sync`, every unattended pass).
    Both,
    /// Push only (`--action push`, the inline push after a save).
    Push,
    /// Pull only (`--action pull`).
    Pull,
}

impl Legs {
    /// Whether the pass pulls under `mode` (the inline push never does).
    #[must_use]
    pub const fn pulls(self, mode: Mode) -> bool {
        !matches!(self, Self::Push) && !matches!(mode, Mode::Inline(_))
    }

    /// Whether the pass pushes.
    #[must_use]
    pub const fn pushes(self) -> bool {
        !matches!(self, Self::Pull)
    }
}

/// A pass ready to use the network.
#[derive(Debug)]
pub struct Session {
    /// The key every row is filed under.
    pub key: ExchangeKey,
    /// The key's state, as this pass will write it back.
    pub row: ExchangeRow,
    /// The credential's fingerprint, for a suspension.
    pub fingerprint: String,
    /// Replica calls (managed when the origin is).
    pub replica: Transport,
    /// Legacy calls (managed when the origin is).
    pub legacy: Transport,
    /// The policy the pass runs under.
    pub policy: RepositoryPolicy,
    /// The protocol selected for the pass.
    pub protocol: Protocol,
    /// Whether this pass selected `replica-v1` for the first time.
    pub upgraded: bool,
    /// The replica manifest, for a replica pass.
    pub manifest: Option<ManifestResponse>,
}

/// How opening a pass ended.
#[derive(Debug)]
pub enum Opened {
    /// Proceed.
    Ready(Box<Session>),
    /// No request this pass; the row says why.
    Skipped(Box<ExchangeRow>),
}

/// Open a pass for `auth` under `mode`.
///
/// # Errors
/// Configuration, clock and SQLite failures. Network failures are recorded on
/// the key's row and end in [`Opened::Skipped`].
pub fn open(conn: &mut Connection, cfg: &Config, auth: &AuthFile, mode: Mode) -> Result<Opened> {
    let key = ExchangeKey::new(&auth.api_url, &auth.workspace_id);
    let secret = auth.effective_secret();
    let fingerprint =
        sha256_hex(format!("{}\0{}\0{secret}", key.api_url, key.workspace_id).as_bytes());
    let row = sync_exchange::load(conn, &key)?.unwrap_or_else(|| ExchangeRow::fresh(&key));
    if network::gated(&row, &fingerprint, mode == Mode::Manual)? {
        return Ok(Opened::Skipped(Box::new(row)));
    }
    keying::apply_last_used_rule(conn, &key, &network::now()?)?;
    let plain = transport(cfg, &key, &secret, mode)?;
    let (policy, managed) = match load_policy(conn, &plain, auth, &key)? {
        Ok(loaded) => loaded,
        Err(failure) => return skip(conn, row, &failure, &fingerprint),
    };
    let (replica, legacy) = split(plain, managed.then(|| policy.revision()));
    let (outcome, manifest) = read_manifest(&replica);
    let stored = Protocol::parse(row.protocol.as_deref());
    let failure = match negotiate::decide(stored, &outcome) {
        Decision::Select {
            protocol,
            coverage_reason,
            upgraded,
        } => {
            let row = select(
                conn,
                row,
                stored,
                (protocol, coverage_reason, upgraded),
                manifest.as_ref(),
            )?;
            return Ok(Opened::Ready(Box::new(Session {
                key,
                row,
                fingerprint,
                replica,
                legacy,
                policy,
                protocol,
                upgraded,
                manifest: manifest.filter(|_| protocol == Protocol::Replica),
            })));
        }
        Decision::Suspend(status) => Failure::Auth(status),
        Decision::ProtocolError(why) => Failure::Protocol(why),
        Decision::Unavailable(why) => Failure::Unavailable(why),
    };
    skip(conn, row, &failure, &fingerprint)
}

/// The plain transport a pass under `mode` uses: the inline push keeps its
/// own small timeout and makes a single attempt per request.
fn transport(cfg: &Config, key: &ExchangeKey, secret: &str, mode: Mode) -> Result<Transport> {
    Ok(match mode {
        Mode::Inline(timeout) => Transport::new(&key.api_url, secret, timeout)?.without_retries(),
        Mode::Manual | Mode::Unattended => Transport::new(
            &key.api_url,
            secret,
            parse_duration(&cfg.sync.request_timeout)?,
        )?,
    })
}

/// The replica and legacy transports: managed at `revision` when the origin
/// is, plain otherwise.
fn split(plain: Transport, revision: Option<i64>) -> (Transport, Transport) {
    match revision {
        Some(revision) => (
            plain.clone().managed(REPLICA, revision),
            plain.managed(SYNC_PROTOCOL, revision),
        ),
        None => (plain.clone(), plain),
    }
}

/// The policy source: a managed origin's status route, or — when the origin
/// has none (`404`) — the snapshot persisted for this key.
fn load_policy(
    conn: &mut Connection,
    plain: &Transport,
    auth: &AuthFile,
    key: &ExchangeKey,
) -> Result<std::result::Result<(RepositoryPolicy, bool), Failure>> {
    match plain.get::<SyncPolicyStatus>("/v1/sync/status", &[]) {
        Ok(status) => Ok(RepositoryPolicy::from_status(conn, status, auth)
            .map(|policy| (policy, true))
            .map_err(|e| Failure::Protocol(format!("sync policy: {e}")))),
        Err(Failure::NotFound) => Ok(Ok((RepositoryPolicy::from_snapshot(conn, key)?, false))),
        Err(failure) => Ok(Err(failure)),
    }
}

/// Record the selection on the row, and begin an upgrade the first time
/// `replica-v1` is selected.
fn select(
    conn: &Connection,
    mut row: ExchangeRow,
    stored: Option<Protocol>,
    (protocol, coverage_reason, upgraded): (Protocol, Option<&'static str>, bool),
    manifest: Option<&ManifestResponse>,
) -> Result<ExchangeRow> {
    let now = network::now()?;
    row.protocol = Some(protocol.as_str().to_string());
    row.coverage_reason = coverage_reason.map(str::to_string);
    if stored != Some(protocol) {
        row.selected_at = Some(now.clone());
    }
    if upgraded {
        let head = manifest.map_or(0, |m| m.head_sequence);
        upgrade::begin(conn, &mut row, head, &now)?;
    }
    Ok(row)
}

/// Read and classify the replica manifest.
fn read_manifest(replica: &Transport) -> (Manifest, Option<ManifestResponse>) {
    match replica.get::<ManifestResponse>("/v1/sync/replica/manifest", &[]) {
        Ok(manifest) if manifest.protocol != REPLICA => (
            Manifest::Invalid(format!("the upstream speaks {}", manifest.protocol)),
            None,
        ),
        Ok(manifest) if manifest.capabilities.iter().any(|c| c == REPLICA) => {
            (Manifest::Ready, Some(manifest))
        }
        // A seeding engine advertises nothing at all; capabilities that name
        // kinds but not the protocol are not a replica manifest.
        Ok(manifest) if manifest.capabilities.is_empty() => (Manifest::Seeding, Some(manifest)),
        Ok(_) => (
            Manifest::Invalid(format!("the upstream does not advertise {REPLICA}")),
            None,
        ),
        Err(Failure::NotFound) => (Manifest::Missing, None),
        Err(Failure::Auth(status)) => (Manifest::Unauthorized(status), None),
        Err(Failure::Unavailable(why)) => (Manifest::Unavailable(why), None),
        Err(Failure::RateLimited(_)) => (Manifest::Unavailable("rate limited".into()), None),
        Err(other) => (Manifest::Invalid(format!("{other:?}")), None),
    }
}

/// The time after which a pass starts no new batch: `[sync] pass_budget`
/// for unattended passes, the inline timeout for the push after a save, none
/// for a person's run.
///
/// # Errors
/// [`Error::Config`] for an unparsable `pass_budget`.
pub fn budget(cfg: &Config, mode: Mode) -> Result<Option<Duration>> {
    Ok(match mode {
        Mode::Manual => None,
        Mode::Unattended => Some(parse_duration(&cfg.sync.pass_budget)?),
        Mode::Inline(timeout) => Some(timeout),
    })
}

/// Close a pass: record how it ended on the key and save the key's row.
///
/// A policy that changed under the pass is not recorded: the drain reloads
/// it and runs once more at once.
///
/// # Errors
/// Clock and SQLite failures.
pub fn close(
    conn: &Connection,
    row: &mut ExchangeRow,
    (failure, fingerprint): (Option<&Failure>, &str),
    report: &mut Report,
    at: &str,
) -> Result<()> {
    match failure {
        Some(Failure::Conflict(code)) if code == network::POLICY_CHANGED => {
            report.policy_changed = true;
            report.end = End::Network;
        }
        Some(failure) => {
            network::fail(row, failure, fingerprint)?;
            report.end = End::Network;
            report.more = false;
        }
        None => network::succeed(row)?,
    }
    row.last_session_at = Some(at.to_string());
    sync_exchange::save(conn, row, at)?;
    report.network.clone_from(&row.network_state);
    Ok(())
}

/// Record `failure` on the key and end the pass without a request.
fn skip(
    conn: &Connection,
    mut row: ExchangeRow,
    failure: &Failure,
    fingerprint: &str,
) -> Result<Opened> {
    network::fail(&mut row, failure, fingerprint)?;
    row.last_session_at = Some(network::now()?);
    sync_exchange::save(conn, &row, &network::now()?)?;
    Ok(Opened::Skipped(Box::new(row)))
}

#[cfg(test)]
#[path = "tests/session.rs"]
mod tests;
