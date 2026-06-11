//! Golden-set model: hand-written YAML pairs + pairs harvested from
//! feedback provenance, merged (file wins on duplicate query text).

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::prelude::*;

/// One golden evaluation pair: a query and the memory ids a correct
/// retrieval should surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenPair {
    /// Query text, run verbatim through the lexical pipeline.
    pub query: String,
    /// Memory ids considered relevant for the query.
    pub relevant: Vec<String>,
}

/// Load golden pairs from a YAML file (`- query: ...` / `  relevant: [..]`).
pub fn load_file(path: &Path) -> Result<Vec<GoldenPair>> {
    let raw = std::fs::read_to_string(path).map_err(Error::Io)?;
    serde_yaml::from_str(&raw)
        .map_err(|e| Error::Config(format!("golden file {}: {e}", path.display())))
}

/// Harvest golden pairs from feedback provenance: every distinct query
/// text in `retrieval_log` paired with the ids marked `used` for it.
/// Ids whose memory row is gone or soft-deleted are dropped; queries
/// left with zero live relevant ids are omitted. BTreeMap keeps the
/// output deterministic.
pub fn harvest(conn: &Connection) -> Result<Vec<GoldenPair>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT r.query, e.memory_id
           FROM feedback_events e
           JOIN retrieval_log r ON r.query_id = e.query_id
           JOIN memories m ON m.id = e.memory_id AND m.deleted_at IS NULL
          WHERE e.verdict = 'used'
          ORDER BY r.query, e.memory_id",
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut by_query: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (query, id) in rows {
        by_query.entry(query).or_default().push(id);
    }
    Ok(by_query
        .into_iter()
        .map(|(query, relevant)| GoldenPair { query, relevant })
        .collect())
}

/// Merge file pairs over harvested pairs: identical query text → the
/// file pair wins (hand-curated truth beats inferred truth). Output is
/// sorted by query text for deterministic eval order.
pub fn merge(file: Vec<GoldenPair>, harvested: Vec<GoldenPair>) -> Vec<GoldenPair> {
    let mut by_query: BTreeMap<String, GoldenPair> = BTreeMap::new();
    for p in harvested {
        by_query.insert(p.query.clone(), p);
    }
    for p in file {
        by_query.insert(p.query.clone(), p);
    }
    by_query.into_values().collect()
}

/// Resolve the effective golden set for eval/tune: optional file pairs
/// merged over the harvest (file wins). `golden_only` skips the
/// harvest. Errors with [`Error::Unavailable`] when the result is
/// empty — scoring zero pairs is meaningless and the caller should
/// know why.
pub fn resolve(
    conn: &Connection,
    golden_file: Option<&Path>,
    golden_only: bool,
) -> Result<Vec<GoldenPair>> {
    let file_pairs = match golden_file {
        Some(p) => load_file(p)?,
        None => Vec::new(),
    };
    let harvested = if golden_only {
        Vec::new()
    } else {
        harvest(conn)?
    };
    let pairs = merge(file_pairs, harvested);
    if pairs.is_empty() {
        return Err(Error::Unavailable(
            "no golden pairs: record feedback (comemory feedback <query_id> --used <ids>) \
             or pass --golden <file>"
                .into(),
        ));
    }
    Ok(pairs)
}
