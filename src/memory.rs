/// YAML frontmatter struct plus markdown split/render helpers.
pub mod frontmatter;
/// Deterministic 8-hex memory id derived from the body content hash.
pub mod id;
/// What the markdown tree already holds for an id (`Prior`), consulted
/// before a save.
pub mod prior;
/// Versioned code references (`Ref`) with string-or-struct serde.
pub mod references;
/// Filesystem-safe slug derivation for memory filenames.
pub mod slug;
/// Markdown-backed memory store: save / load / list / soft-delete.
pub mod store;

pub use frontmatter::{Frontmatter, Kind, References, Relations};
pub use prior::Prior;
pub use references::Ref;
pub use store::{MemoryRecord, MemoryStore, SaveParams};
