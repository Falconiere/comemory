//! Shared command core between `cli::` and `serve::routes::`:
//! `api::<cmd>::run(&mut Ctx, Request)` holds each subcommand's logic, so
//! neither surface duplicates it (precedent:
//! `retrieval::code_search::search_code_hits`).
//!
//! The execution context itself is transport-neutral and lives in
//! [`crate::utilities::context::Ctx`]; this module is the staged shell that
//! [#178](https://github.com/Falconiere/comemory/issues/178) removes once every
//! core has moved under `domains::`.

/// `comemory consolidate`: advisory near-duplicate cluster report.
pub mod consolidate;
/// `comemory doctor`: runtime health check.
pub mod doctor;
/// `comemory gc`: trash sweep + learning-telemetry retention purge.
pub mod gc;
/// `comemory install`: bundled agent skills and hooks for a host.
pub mod install;
/// `comemory prune`: orphan / low-value / stale-code candidates, dry-run
/// report plus (CLI-driven) apply.
pub mod prune;
/// `comemory rebuild`: atomically rebuild the SQLite mirror from markdown.
pub mod rebuild;
/// `comemory setup`: detect, plan, and apply first-run onboarding.
pub mod setup;
/// `comemory stats`: corpus counters and database size.
pub mod stats;

// Console-only cores (console-api spec, 2026-09-01): no CLI subcommand of
// their own, reached through `serve::routes`.

/// `GET|PUT /gc/policy`: retention windows + last gc run.
pub mod gc_policy;
/// `GET /overview`, `GET /overview/eval-series`.
pub mod overview;
/// `POST /doctor/reembed`: re-vectorize through the embed command.
pub mod reembed;
