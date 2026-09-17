//! `domains::memories` — the memory lifecycle, end to end.
//!
//! Markdown is the source of truth: [`frontmatter`], [`id`], [`slug`],
//! [`references`] and [`prior`] model a record and [`store`] writes it
//! atomically. The command cores both delivery adapters call sit beside them —
//! [`save`], [`delete`], [`list`], [`show`], [`update`], [`restore`],
//! [`trash`] and [`refresh_refs`] — with [`nav`] holding the two derived
//! fields every listing reports. SQLite stays in the central `store`; clap
//! flags, process I/O and the inline cloud push stay in `cli`.

/// `comemory delete` / `DELETE /memories/{id}`, plus the soft-delete helpers
/// `comemory prune` and the sync import share.
pub mod delete;
/// YAML frontmatter struct plus markdown split/render helpers.
pub mod frontmatter;
/// Deterministic 8-hex memory id derived from the body content hash.
pub mod id;
/// `comemory list` / `GET /memories`: page live memories.
pub mod list;
/// What the markdown tree already holds for an id (`Prior`), consulted
/// before a save.
pub mod prior;
/// Versioned code references (`Ref`) with string-or-struct serde.
pub mod references;
/// `POST /memories/{id}/references/refresh`: re-pin anchors to HEAD.
pub mod refresh_refs;
/// `POST /memories/{id}/restore`, `POST /trash/{id}/restore`.
pub mod restore;
/// `comemory save` / `POST /memories`: write a memory and mirror it.
pub mod save;
/// `comemory show` / `GET /memories/{id}`: one memory in full.
pub mod show;
/// Filesystem-safe slug derivation for memory filenames.
pub mod slug;
/// Markdown-backed memory store: save / load / list / soft-delete.
pub mod store;
/// `GET /trash`: soft-deleted memories with their days until gc.
pub mod trash;
/// `PATCH /memories/{id}`: frontmatter patch or superseding re-save.
pub mod update;

/// The two derived navigation fields a memory listing reports: its title and
/// the absolute path of its markdown file.
pub(crate) mod nav;

pub use frontmatter::{Frontmatter, Kind, References, Relations};
pub use prior::Prior;
pub use references::Ref;
pub use store::{MemoryRecord, MemoryStore, SaveParams};
