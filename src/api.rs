//! Shared command core between `cli::` and `serve::routes::`:
//! `api::<cmd>::run(&mut Ctx, Request)` holds each subcommand's logic, so
//! neither surface duplicates it (precedent:
//! `retrieval::code_search::search_code_hits`).
//!
//! The execution context itself is transport-neutral and lives in
//! [`crate::utilities::context::Ctx`]; this module is the staged shell that
//! [#178](https://github.com/Falconiere/comemory/issues/178) removes once every
//! core has moved under `domains::`.

/// `comemory bandit`: Thompson-sample the `[tune]` grid, confirm, apply.
pub mod bandit;
/// `comemory consolidate`: advisory near-duplicate cluster report.
pub mod consolidate;
/// `comemory context`: headline memory + code bundle for a query.
pub mod context;
/// `comemory doctor`: runtime health check.
pub mod doctor;
/// `comemory edges`: lexical search over the relation graph.
pub mod edges;
/// `comemory eval`: score retrieval quality against a golden set.
pub mod eval;
/// `comemory feedback`: record which hits were used.
pub mod feedback;
/// `comemory find`: one ranked list across memory, code, and documents.
pub mod find;
/// `comemory gc`: trash sweep + learning-telemetry retention purge.
pub mod gc;
/// `comemory graph`: the file-level code-connection graph, full or paged.
pub mod graph;
/// `comemory install`: bundled agent skills and hooks for a host.
pub mod install;
/// `comemory mine`: distill query-reformulation term mappings.
pub mod mine;
/// `comemory prune`: orphan / low-value / stale-code candidates, dry-run
/// report plus (CLI-driven) apply.
pub mod prune;
/// `comemory rebuild`: atomically rebuild the SQLite mirror from markdown.
pub mod rebuild;
/// `comemory search`: hybrid memory retrieval.
pub mod search;
/// `comemory search-code`: ranked code search.
pub mod search_code;
/// `comemory setup`: detect, plan, and apply first-run onboarding.
pub mod setup;
/// `comemory stats`: corpus counters and database size.
pub mod stats;
/// `comemory tune`: grid-search the blend knobs, confirm, apply.
pub mod tune;

// Console-only cores (console-api spec, 2026-09-01): no CLI subcommand of
// their own, reached through `serve::routes`.

/// `GET|PUT /config/retrieval`: live ranking knobs.
pub mod config_retrieval;
/// `GET|PUT /gc/policy`: retention windows + last gc run.
pub mod gc_policy;
/// `GET /graph/nodes*`, `GET /graph/snapshot`: node listing, detail, neighbors.
pub mod graph_nodes;
/// `POST /graph/recompute`: PageRank re-projection job.
pub mod graph_recompute;
/// `GET /learning/{summary,evals,golden-set,expansions}`.
pub mod learning;
/// `GET /learning/proposals`, `POST /learning/proposals/{id}/{apply,discard}`.
pub mod learning_proposals;
/// `GET /overview`, `GET /overview/eval-series`.
pub mod overview;
/// `POST /doctor/reembed`: re-vectorize through the embed command.
pub mod reembed;
/// `GET /search/suggest`: mined expansions + recent queries.
pub mod suggest;
