//! LanceDB-backed vector index for memory bodies. Wraps `connect`, `merge_insert`,
//! and `nearest_to` so the rest of qwick-memory treats memory search as upsert + topK.

use std::path::Path;
use std::sync::Arc;

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Int32Array, RecordBatch, RecordBatchIterator,
    StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::Connection;
use time::format_description::well_known::Iso8601;

use crate::index::schema::{memory_schema, MEMORY_TABLE};
use crate::index::score_from_distance;
use crate::memory::{Kind, MemoryRecord};
use crate::prelude::*;

/// LanceDB connection plus the cached arrow schema we encode rows against.
pub struct MemoryIndex {
    conn: Connection,
    schema: Arc<Schema>,
}

/// A single search result: memory id, similarity score (higher = closer), and
/// enough metadata for callers to render a hit without re-reading from disk.
#[derive(Debug, Clone)]
pub struct MemoryHit {
    pub id: String,
    pub score: f32,
    pub body: String,
    pub kind: Kind,
    pub repo: String,
}

impl MemoryIndex {
    /// Open (or create) the LanceDB database at `dir`. `dim` MUST match the
    /// embedder used for `upsert` and `search`.
    pub async fn open(dir: impl AsRef<Path>, dim: usize) -> Result<Self> {
        let uri = dir.as_ref().to_string_lossy().to_string();
        let conn = lancedb::connect(&uri).execute().await?;
        Ok(Self {
            conn,
            schema: memory_schema(dim),
        })
    }

    /// Upsert one memory: creates the `memory_chunks` table on first call,
    /// then uses `merge_insert(id)` so re-saves overwrite rather than duplicate.
    pub async fn upsert(&self, rec: &MemoryRecord, emb: &[f32]) -> Result<()> {
        let batch = batch_from_record(self.schema.clone(), rec, emb)?;
        let schema = self.schema.clone();
        let names = self.conn.table_names().execute().await?;

        if names.iter().any(|n| n == MEMORY_TABLE) {
            let batches = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
            let tbl = self.conn.open_table(MEMORY_TABLE).execute().await?;
            let mut merge = tbl.merge_insert(&["id"]);
            merge.when_matched_update_all(None);
            merge.when_not_matched_insert_all();
            merge.execute(Box::new(batches)).await?;
        } else {
            let batches = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
            self.conn
                .create_table(MEMORY_TABLE, Box::new(batches) as Box<_>)
                .execute()
                .await?;
        }
        Ok(())
    }

    /// Vector search for `limit` nearest memories. Returns `[]` when the
    /// table doesn't exist yet (first call before any upserts).
    pub async fn search(&self, query_emb: &[f32], limit: usize) -> Result<Vec<MemoryHit>> {
        let names = self.conn.table_names().execute().await?;
        if !names.iter().any(|n| n == MEMORY_TABLE) {
            return Ok(Vec::new());
        }
        let tbl = self.conn.open_table(MEMORY_TABLE).execute().await?;
        let batches: Vec<RecordBatch> = tbl
            .query()
            .nearest_to(query_emb)?
            .limit(limit)
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut hits = Vec::new();
        for b in &batches {
            collect_hits(b, &mut hits)?;
        }
        Ok(hits)
    }
}

/// Encode one `MemoryRecord` + its embedding into a single-row `RecordBatch`
/// matching the schema returned by `memory_schema`.
fn batch_from_record(schema: Arc<Schema>, rec: &MemoryRecord, emb: &[f32]) -> Result<RecordBatch> {
    let fm = &rec.frontmatter;
    let tags_csv = fm.tags.join(",");
    let kind_str = fm.kind.as_str();
    let created_str = fm
        .created
        .format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(e.to_string()))?;
    let item_field = Arc::new(Field::new("item", DataType::Float32, true));
    let values: Arc<dyn Array> = Arc::new(Float32Array::from(emb.to_vec()));
    let emb_array = FixedSizeListArray::try_new(item_field, emb.len() as i32, values, None)
        .map_err(|e| Error::Other(e.to_string()))?;

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![fm.id.clone()])),
            Arc::new(StringArray::from(vec![rec.body.clone()])),
            Arc::new(StringArray::from(vec![kind_str.to_string()])),
            Arc::new(StringArray::from(vec![fm.repo.clone()])),
            Arc::new(StringArray::from(vec![tags_csv])),
            Arc::new(StringArray::from(vec![created_str])),
            Arc::new(Int32Array::from(vec![fm.quality as i32])),
            Arc::new(StringArray::from(vec![fm.content_hash.clone()])),
            Arc::new(emb_array),
        ],
    )
    .map_err(|e| Error::Other(e.to_string()))?;
    Ok(batch)
}

/// Extract `MemoryHit` rows from a result `RecordBatch`. LanceDB returns an
/// extra `_distance` column on vector queries; we convert L2 distance to a
/// monotone `1 / (1 + d)` similarity score so callers can sort descending.
///
/// If the `_distance` column is absent (or has the wrong type), we error
/// rather than silently falling back to a perfect score: a missing column is
/// a schema mismatch the caller must see, not a "every hit is top-rank"
/// regression.
pub fn collect_hits(batch: &RecordBatch, out: &mut Vec<MemoryHit>) -> Result<()> {
    let id_col = downcast_str(batch, "id")?;
    let body_col = downcast_str(batch, "body")?;
    let kind_col = downcast_str(batch, "kind")?;
    let repo_col = downcast_str(batch, "repo")?;
    let dist_col = batch
        .column_by_name("_distance")
        .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
        .ok_or_else(|| Error::Other("missing _distance column".into()))?;

    for i in 0..batch.num_rows() {
        let kind = Kind::parse_or_note(kind_col.value(i));
        let score = score_from_distance(dist_col.value(i));
        out.push(MemoryHit {
            id: id_col.value(i).into(),
            score,
            body: body_col.value(i).into(),
            kind,
            repo: repo_col.value(i).into(),
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
