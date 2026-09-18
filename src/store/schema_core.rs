//! Declared schema — the tables no single domain owns: the migration
//! runner's own `schema_meta` key/value store and the derived `edge_fts`
//! triplet index over `edges`.
//!
//! These structs declare schema metadata and generate column constants and
//! query constructors through their attributes. `registry()` in `schema.rs`
//! assembles the migration input; runtime store helpers in other modules
//! consume the generated columns and constructors through `store::orm`.

use toolu_orm::core::column::Text;
use toolu_orm::{fts5_table, table};

/// `schema_meta`: the migration markers, the locked vector dims, and the
/// per-repo cursors — see `store::schema_meta` for the readers.
#[table(name = "schema_meta")]
pub struct SchemaMeta {
    /// Marker or setting name.
    #[column(primary_key)]
    pub key: Text,
    /// Its value, always text.
    #[column(not_null)]
    pub value: Text,
}

/// `edge_fts`: every `edges` row rendered as searchable `src —rel→ dst`
/// text, with the raw edge carried in `UNINDEXED` payload columns. Refreshed
/// wholesale by `store::edge_fts::refresh`, never written incrementally.
#[fts5_table(name = "edge_fts", tokenize = "identifier")]
pub struct EdgeFts {
    /// Rendered source node text.
    pub src_text: Text,
    /// Rendered relation text.
    pub rel_text: Text,
    /// Rendered destination node text.
    pub dst_text: Text,
    /// Raw `edges.src_kind`.
    #[column(unindexed)]
    pub src_kind: Text,
    /// Raw `edges.src_id`.
    #[column(unindexed)]
    pub src_id: Text,
    /// Raw `edges.rel`.
    #[column(unindexed)]
    pub rel: Text,
    /// Raw `edges.dst_kind`.
    #[column(unindexed)]
    pub dst_kind: Text,
    /// Raw `edges.dst_id`.
    #[column(unindexed)]
    pub dst_id: Text,
    /// Raw `edges.weight`, carried as text by FTS5.
    #[column(unindexed)]
    pub weight: Text,
}
