//! Memory- and code-layer DDL for the kuzu property graph.
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

/// Node + relation table definitions for the code layer.
///
/// The code layer mirrors the on-disk source tree as graph entities: `File`
/// nodes (one per source file, keyed by `<repo>:<path>`) and `Symbol` nodes
/// (one per extracted definition, keyed by `<repo>:<path>:<name>`), connected
/// by intra-code relations (`DefinedIn`, `Calls`, `Imports`) plus the
/// cross-layer back-edges (`ReferencesFile`, `ReferencesSymbol`) that link a
/// `Memory` body to the code artefacts it mentions.
pub const CODE_LAYER_DDL: &[&str] = &[
  "CREATE NODE TABLE IF NOT EXISTS File(qualified STRING, repo STRING, path STRING, content_hash STRING, indexed_at STRING, PRIMARY KEY(qualified))",
  "CREATE NODE TABLE IF NOT EXISTS Symbol(qualified STRING, name STRING, kind STRING, language STRING, ast_hash STRING, PRIMARY KEY(qualified))",
  "CREATE REL TABLE IF NOT EXISTS DefinedIn(FROM Symbol TO File)",
  "CREATE REL TABLE IF NOT EXISTS Calls(FROM Symbol TO Symbol)",
  "CREATE REL TABLE IF NOT EXISTS Imports(FROM File TO File)",
  "CREATE REL TABLE IF NOT EXISTS ReferencesFile(FROM Memory TO File)",
  "CREATE REL TABLE IF NOT EXISTS ReferencesSymbol(FROM Memory TO Symbol)",
];
