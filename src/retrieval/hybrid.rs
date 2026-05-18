//! Memory-layer hybrid search. The "hybrid" name anticipates the FTS branch
//! that lands in a later task; today it is vector-only with a deterministic
//! threshold filter on top of `MemoryIndex::search`.
//!
//! Code-layer search lives here too: `search_code` queries the `code_chunks`
//! LanceDB table directly via the connection borrowed from `CodeIndex`. The
//! two functions share the same shape (`limit * 2` over-fetch, threshold
//! filter, descending sort, truncate) so callers can merge their outputs
//! into a single bundle in later tasks.

use arrow_array::{Float32Array, RecordBatch, StringArray};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};

use crate::index::schema::CODE_TABLE;
use crate::index::{score_from_distance, CodeIndex, MemoryHit, MemoryIndex};
use crate::prelude::*;

/// Vector search for memories.
///
/// Over-fetches `limit * 2` from LanceDB, sorts by score descending, drops
/// anything below `threshold`, and truncates back to `limit`. The over-fetch
/// gives the threshold room to act without starving callers that want a
/// fixed number of results.
pub async fn search_memory(
    index: &MemoryIndex,
    query_emb: &[f32],
    limit: usize,
    threshold: f32,
) -> Result<Vec<MemoryHit>> {
    let mut hits = index.search(query_emb, limit * 2).await?;
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.retain(|h| h.score >= threshold);
    hits.truncate(limit);
    Ok(hits)
}

/// One code-layer search result. Mirrors `MemoryHit`'s shape so the two
/// layers can be presented side-by-side in a bundle without bespoke
/// adapters: `score` is the monotone similarity, the remaining fields
/// give callers enough metadata to render a hit without re-reading from
/// LanceDB.
#[derive(Debug, Clone)]
pub struct CodeHit {
    /// `<repo>:<path>:<symbol>` — primary key of the underlying chunk row.
    pub qualified: String,
    /// `1 / (1 + d)` similarity score (higher is closer), to match
    /// `MemoryHit::score` so merge/sort logic stays uniform.
    pub score: f32,
    /// Source text of the extracted symbol.
    pub snippet: String,
    /// Lower-case language tag (`rust`, `python`, `typescript`, `javascript`).
    pub language: String,
    /// `<repo>:<path>` — denormalized so callers can render a file label
    /// without parsing `qualified`.
    pub file: String,
}

/// Vector search for code symbols. Same over-fetch + threshold + descending
/// sort + truncate shape as `search_memory`; returns an empty vector when
/// the `code_chunks` table doesn't exist yet (no `index_repo` has run).
pub async fn search_code(
    index: &CodeIndex,
    query_emb: &[f32],
    limit: usize,
    threshold: f32,
) -> Result<Vec<CodeHit>> {
    let names = index.conn().table_names().execute().await?;
    if !names.iter().any(|n| n == CODE_TABLE) {
        return Ok(Vec::new());
    }
    let tbl = index.conn().open_table(CODE_TABLE).execute().await?;
    let batches: Vec<RecordBatch> = tbl
        .query()
        .nearest_to(query_emb)?
        .limit(limit * 2)
        .execute()
        .await?
        .try_collect()
        .await?;

    let mut out = Vec::new();
    for batch in &batches {
        collect_code_hits(batch, threshold, &mut out)?;
    }
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(limit);
    Ok(out)
}

/// Decode one `code_chunks` result `RecordBatch` into `CodeHit` rows. The
/// `_distance` column is converted to a `1 / (1 + d)` similarity score so
/// `threshold` comparisons read the same way for memory and code layers.
///
/// Missing `_distance` is treated as a schema mismatch — we error rather
/// than silently scoring every hit as `1.0`, mirroring `memory_index::collect_hits`.
fn collect_code_hits(batch: &RecordBatch, threshold: f32, out: &mut Vec<CodeHit>) -> Result<()> {
    let qualified = downcast_str(batch, "qualified")?;
    let snippet = downcast_str(batch, "snippet")?;
    let language = downcast_str(batch, "language")?;
    let file = downcast_str(batch, "file")?;
    let dist = batch
        .column_by_name("_distance")
        .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
        .ok_or_else(|| Error::Other("missing _distance column".into()))?;

    for i in 0..batch.num_rows() {
        let score = score_from_distance(dist.value(i));
        if score < threshold {
            continue;
        }
        out.push(CodeHit {
            qualified: qualified.value(i).into(),
            score,
            snippet: snippet.value(i).into(),
            language: language.value(i).into(),
            file: file.value(i).into(),
        });
    }
    Ok(())
}

fn downcast_str<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a StringArray> {
    batch
        .column_by_name(name)
        .ok_or_else(|| Error::Other(format!("missing column: {name}")))?
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| Error::Other(format!("column not StringArray: {name}")))
}
