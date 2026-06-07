//! Code-layer vector search. The memory-layer equivalent has moved into
//! `retrieval::fuse::search_memory_fused_with_fts`; pass `fts = None` to
//! get pure dense retrieval without re-implementing the over-fetch and
//! threshold logic.
//!
//! `search_code` queries the `code_chunks` LanceDB table directly via the
//! connection borrowed from `CodeIndex`.

use arrow_array::{Float32Array, RecordBatch, StringArray};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};

use crate::index::schema::CODE_TABLE;
use crate::index::{score_from_distance, CodeIndex};
use crate::prelude::*;

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
