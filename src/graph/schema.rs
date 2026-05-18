//! Memory-layer DDL for the kuzu property graph.
//!
//! Each statement is idempotent (`IF NOT EXISTS`) so [`crate::graph::Graph::open`]
//! can replay the full list on every startup without churn.

/// Node + relation table definitions for the memory layer.
///
/// The memory layer captures human-curated knowledge: `Memory` nodes linked to
/// `Repo`, `Author`, and `Tag` provenance plus the cross-memory relations
/// (`Supersedes`, `ConflictsWith`, `RelatesTo`, `DerivedFrom`) used by the
/// retrieval pipeline.
pub const MEMORY_LAYER_DDL: &[&str] = &[
  "CREATE NODE TABLE IF NOT EXISTS Memory(id STRING, kind STRING, created STRING, quality INT64, PRIMARY KEY(id))",
  "CREATE NODE TABLE IF NOT EXISTS Repo(name STRING, last_indexed_head STRING, last_indexed_at STRING, PRIMARY KEY(name))",
  "CREATE NODE TABLE IF NOT EXISTS Author(name STRING, PRIMARY KEY(name))",
  "CREATE NODE TABLE IF NOT EXISTS Tag(name STRING, PRIMARY KEY(name))",
  "CREATE REL TABLE IF NOT EXISTS InRepo(FROM Memory TO Repo)",
  "CREATE REL TABLE IF NOT EXISTS AuthoredBy(FROM Memory TO Author)",
  "CREATE REL TABLE IF NOT EXISTS Tagged(FROM Memory TO Tag)",
  "CREATE REL TABLE IF NOT EXISTS Supersedes(FROM Memory TO Memory, at STRING)",
  "CREATE REL TABLE IF NOT EXISTS ConflictsWith(FROM Memory TO Memory)",
  "CREATE REL TABLE IF NOT EXISTS RelatesTo(FROM Memory TO Memory, score DOUBLE)",
  "CREATE REL TABLE IF NOT EXISTS DerivedFrom(FROM Memory TO Memory)",
];
