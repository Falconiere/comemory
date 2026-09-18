//! The integrations capability: getting comemory working inside an agent host,
//! and getting a machine or a repo ready to use it.
//!
//! `install` extracts the embedded agent bundle into a versioned local
//! marketplace and registers it with the host's own plugin manager,
//! connection-free, so installing never creates a database. `setup` composes
//! detect → plan → apply on top, delegating to `domains::maintenance` for the
//! doctor report, `domains::code` for hooks, repositories and code indexing,
//! `domains::documents` for the source listing and `domains::sync` for the
//! local credential. Its seven step ids are a public contract. Both are
//! CLI-only — a server must never write into an operator's agent config — and
//! the authored assets stay in the repository-root `integrations/agent/`.

/// `comemory install`: the embedded agent bundle and the host registration.
pub mod install;
/// `comemory setup`: detect, plan, and apply first-run onboarding.
pub mod setup;
