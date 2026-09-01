mod defaults;
/// `COMEMORY_*` env-var overrides — the outermost config layer.
pub mod env;
/// Struct definitions, shipped defaults, and the `config.toml` overlay.
pub mod file;
/// Learning-loop sections: `[tune]` grids, `[reinforce]`, `[bandit]`.
pub mod learning;
/// The one read-patch-write primitive over `config.toml`.
pub mod patch;
/// Data-directory layout resolution.
pub mod paths;
/// The `[retrieval]` section and its file overlay.
pub mod retrieval;
mod validate;

pub use file::{AutoReindexMode, Config};
pub use learning::{BanditConfig, ReinforceConfig, TuneConfig};
pub use paths::Paths;
pub use retrieval::RetrievalConfig;
