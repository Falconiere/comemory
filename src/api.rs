//! Shared command core between `cli::` and `serve::routes::`:
//! `api::<cmd>::run(&mut Ctx, Request)` holds each subcommand's logic, so
//! neither surface duplicates it (precedent:
//! `retrieval::code_search::search_code_hits`).
//!
//! The execution context itself is transport-neutral and lives in
//! [`crate::utilities::context::Ctx`]; this module is the staged shell that
//! [#178](https://github.com/Falconiere/comemory/issues/178) removes once every
//! core has moved under `domains::`. Only the integrations cores are left —
//! [#175](https://github.com/Falconiere/comemory/issues/175) takes them.

/// `comemory install`: bundled agent skills and hooks for a host.
pub mod install;
/// `comemory setup`: detect, plan, and apply first-run onboarding.
pub mod setup;
