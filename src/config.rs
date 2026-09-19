mod defaults;
/// `COMEMORY_*` env-var overrides — the outermost config layer.
pub mod env;
/// Struct definitions, shipped defaults, and the `config.toml` overlay.
pub mod file;
/// Learning-loop sections: `[tune]` grids, `[reinforce]`, `[bandit]`.
pub mod learning;
/// The `[observations]` section: opt-in candidate observation capture.
pub mod observations;
/// The one read-patch-write primitive over `config.toml`.
pub mod patch;
/// Data-directory layout resolution.
pub mod paths;
/// The `[prune]` section: scoring floors and retention windows.
pub mod prune;
/// The `[rerank]` section: the opt-in learned ordering stage.
pub mod rerank;
/// The `[retrieval]` section and its file overlay.
pub mod retrieval;
/// The `[sync]` and `[embed]` sections.
pub mod sync;
mod validate;

pub use file::{AutoReindexMode, Config};
pub use learning::{BanditConfig, ReinforceConfig, TuneConfig};
pub use observations::ObservationsConfig;
pub use paths::Paths;
pub use prune::PruneConfig;
pub use rerank::RerankConfig;
pub use retrieval::RetrievalConfig;
pub use sync::{EmbedConfig, SyncConfig};
