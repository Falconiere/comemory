//! Golden-set model: hand-written YAML pairs + pairs harvested from
//! feedback provenance, merged (file wins on duplicate
//! (query, repo, kind) key).

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
    /// Repo filter the originating search used, replayed by eval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Kind filter the originating search used, replayed by eval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// Identity of a golden pair for harvest aggregation and merge:
/// (query, repo, kind). The same query text searched under different
/// filters is a different retrieval problem and stays a distinct pair.
type PairKey = (String, Option<String>, Option<String>);

/// Lift a pair's [`PairKey`] out by clone.
fn pair_key(p: &GoldenPair) -> PairKey {
    (p.query.clone(), p.repo.clone(), p.kind.clone())
}

/// Load golden pairs from a YAML file (`- query: ...` / `  relevant: [..]`).
pub fn load_file(path: &Path) -> Result<Vec<GoldenPair>> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| Error::Config(format!("golden file {}: {e}", path.display())))?;
    serde_yaml::from_str(&raw)
        .map_err(|e| Error::Config(format!("golden file {}: {e}", path.display())))
}

/// Harvest golden pairs from feedback provenance: every distinct
/// (query, repo, kind) triple in `retrieval_log` paired with the ids
/// marked `used` for it — the originating filters travel with the pair
/// so eval can replay them faithfully. Ids whose memory row is gone or
/// soft-deleted are dropped; keys left with zero live relevant ids are
/// omitted. BTreeMap keeps the output deterministic.
pub fn harvest(conn: &Connection) -> Result<Vec<GoldenPair>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT r.query, r.repo, r.kind, e.memory_id
           FROM feedback_events e
           JOIN retrieval_log r ON r.query_id = e.query_id
           JOIN memories m ON m.id = e.memory_id AND m.deleted_at IS NULL
          WHERE e.verdict = 'used'
          ORDER BY r.query, r.repo, r.kind, e.memory_id",
    )?;
    let rows: Vec<(PairKey, String)> = stmt
        .query_map([], |r| Ok(((r.get(0)?, r.get(1)?, r.get(2)?), r.get(3)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut by_key: BTreeMap<PairKey, Vec<String>> = BTreeMap::new();
    for (key, id) in rows {
        by_key.entry(key).or_default().push(id);
    }
    Ok(by_key
        .into_iter()
        .map(|((query, repo, kind), relevant)| GoldenPair {
            query,
            relevant,
            repo,
            kind,
        })
        .collect())
}

/// Merge file pairs over harvested pairs: identical (query, repo, kind)
/// key → the file pair wins (hand-curated truth beats inferred truth).
/// A file pair under different filters is a different retrieval problem
/// and coexists with the harvested one. Output is sorted by key for
/// deterministic eval order.
pub fn merge(file: Vec<GoldenPair>, harvested: Vec<GoldenPair>) -> Vec<GoldenPair> {
    let mut by_key: BTreeMap<PairKey, GoldenPair> = BTreeMap::new();
    for p in harvested.into_iter().chain(file) {
        by_key.insert(pair_key(&p), p);
    }
    by_key.into_values().collect()
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
