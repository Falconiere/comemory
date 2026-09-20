//! The architecture model: one component-level picture of a repository,
//! seeded from the mined code graph, enriched by an agent, and stored as a
//! single memory tagged `architecture` so the console can draw it with any
//! graph library.
//!
//! The domain owns the model and its rules; `cli::architecture` owns the
//! flags and the rendering. Nothing here reaches for an LLM — `learn` runs
//! the command its caller passed and validates what comes back.

/// The versioned model value and its ceilings.
pub mod model;

/// Every rule a model must satisfy before it may be saved.
pub mod validate;

/// Directory clustering, id derivation, and README-seeded summaries.
pub mod cluster;

/// The deterministic scaffold mined from the indexed code graph.
pub mod scaffold;

/// Reading back the model currently in force for a repo.
pub mod current;

/// Validating and storing a model as a tagged memory.
pub mod save;

/// Drift between the saved model and today's index.
pub mod check;

/// Mermaid `flowchart` rendering.
pub mod mermaid;

/// The prompt `learn` hands to an agent.
pub mod prompt;

/// Finding the model in an agent's stdout.
pub mod extract;

/// The `learn` wrapper around a caller-named agent command.
pub mod learn;
