/// YAML frontmatter struct plus markdown split/render helpers.
pub mod frontmatter;
/// Deterministic 8-hex memory id derived from the body content hash.
pub mod id;
/// Versioned code references (`Ref`) with string-or-struct serde.
pub mod references;
/// Filesystem-safe slug derivation for memory filenames.
pub mod slug;
/// Markdown-backed memory store: save / load / list / soft-delete.
pub mod store;

pub use frontmatter::{Frontmatter, Kind, References, Relations};
pub use references::Ref;
pub use store::{MemoryRecord, MemoryStore, SaveParams};
