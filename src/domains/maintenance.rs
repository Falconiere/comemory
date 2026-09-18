//! Operational corpus maintenance: keeping the store healthy, bounded and
//! repairable, and keeping the binary current.
//!
//! Four concerns share one subject — the installation rather than any memory
//! in it. Health probes without creating the file it inspects. Retention
//! keeps the advisory query separate from the confirmed mutation: detection
//! is read-only by construction, only `--apply` writes. Repair rebuilds the
//! mirror from the markdown of record. Upgrade replaces this binary, and is
//! the one command with no HTTP surface at all.
//!
//! Every operation is a separate named file: maintenance is a capability,
//! not a drawer. No SQL lives here — the `ATTACH`/copy/`DETACH` unit and the
//! migration chain belong to [`crate::store`], which this capability
//! composes, as the dashboards compose the cores that own each fact.

/// `comemory consolidate`: the advisory near-duplicate cluster core.
pub mod consolidate;
/// Union-find clustering of live fingerprints and keeper ordering.
pub mod consolidation;
/// The owned value `comemory consolidate` produces.
pub mod consolidation_report;
/// `comemory doctor`: runtime health check, and the read-only probe setup
/// detection consults once a database exists.
pub mod doctor;
/// `comemory gc`: trash sweep plus the learning-telemetry retention purge.
pub mod gc;
/// `GET|PUT /gc/policy`: retention windows and the last gc run.
pub mod gc_policy;
/// `GET /overview`, `GET /overview/eval-series`: the console landing
/// aggregate, composed from the cores that own each fact.
pub mod overview;
/// `comemory prune`: the orphan / low-value / stale-code core — dry-run
/// report plus the confirmed apply.
pub mod prune;
/// `comemory rebuild`: atomically rebuild the SQLite mirror from markdown.
pub mod rebuild;
/// `POST /doctor/reembed`: re-vectorize through the embed command.
pub mod reembed;
/// Stale-memory and ghost-reference detection: read-only by construction.
pub mod retention;
/// The owned value `comemory prune` produces.
pub mod retention_report;
/// `comemory stats`: corpus counters and database size.
pub mod stats;
/// `comemory upgrade`: resolve the newest release and swap this binary.
/// CLI-only — a server must never replace its own binary on request.
pub mod upgrade;
