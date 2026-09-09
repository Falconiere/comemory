//! `GET /sync/changes` — pull log entries above a cursor.

use crate::api::Ctx;
use crate::api::sync::{
    ChangesResponse, SyncEntry, SyncOp, SyncRecord, SyncVector, WireFrontmatter,
};
use crate::memory::{Frontmatter, MemoryStore};
use crate::prelude::*;
use crate::store::{Connection, embed, schema_meta, sync_log, vector};

const MIN_LIMIT: usize = 1;
const MAX_LIMIT: usize = 500;

/// Return up to `limit` sync-log rows with `seq > since`, enriched with live
/// record payloads for upsert/restore ops.
pub fn run(ctx: &mut Ctx<'_>, since: i64, limit: usize) -> Result<ChangesResponse> {
    let limit = limit.clamp(MIN_LIMIT, MAX_LIMIT);
    let paths = ctx.paths.clone();
    let conn = ctx.conn()?;
    let rows = sync_log::entries_since(conn, since, limit)?;
    let head_seq = sync_log::head_seq(conn)?;
    let store = MemoryStore::new(paths);
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let record = match row.op {
            SyncOp::Tombstone => None,
            SyncOp::Upsert | SyncOp::Restore => enrich_record(&store, conn, &row.memory_id)?,
        };
        let author = author_for(&store, &row.memory_id).unwrap_or_default();
        entries.push(SyncEntry {
            seq: row.seq,
            op: row.op,
            id: row.memory_id,
            content_hash: row.content_hash,
            at: row.at,
            author,
            record,
        });
    }
    let next_seq = entries.last().map(|entry| entry.seq);
    Ok(ChangesResponse {
        entries,
        next_seq,
        head_seq,
    })
}

fn author_for(store: &MemoryStore, id: &str) -> Option<String> {
    store.load(id).ok().map(|rec| rec.frontmatter.author)
}

pub(crate) fn enrich_record(
    store: &MemoryStore,
    conn: &Connection,
    id: &str,
) -> Result<Option<SyncRecord>> {
    let rec = match store.load(id) {
        Ok(rec) => rec,
        Err(Error::NotFound(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let vector = vector_for_memory(conn, id)?;
    Ok(Some(record_from_memory(&rec, vector)))
}

/// Build a wire record from a live markdown row, optionally attaching a vector
/// read from `memory_vec`.
pub(crate) fn record_from_memory(
    rec: &crate::memory::MemoryRecord,
    vector: Option<SyncVector>,
) -> SyncRecord {
    SyncRecord {
        frontmatter: wire_frontmatter(&rec.frontmatter),
        body: rec.body.clone(),
        vector,
    }
}

/// Strip `author` for the wire frontmatter object.
pub(crate) fn wire_frontmatter(fm: &Frontmatter) -> WireFrontmatter {
    WireFrontmatter {
        id: fm.id.clone(),
        kind: fm.kind,
        repo: fm.repo.clone(),
        tags: fm.tags.clone(),
        created: fm.created,
        quality: fm.quality,
        schema: fm.schema,
        content_hash: fm.content_hash.clone(),
        references: fm.references.clone(),
        relations: fm.relations.clone(),
    }
}

/// Encode a memory vector row for the wire when a model is configured.
pub(crate) fn vector_for_memory(conn: &Connection, memory_id: &str) -> Result<Option<SyncVector>> {
    let model = schema_meta::memory_vector_model(conn)?;
    if model.is_empty() {
        return Ok(None);
    }
    let dim = vector::dim_memory(conn)?;
    let Some(blob) = vector::memory_embedding_blob(conn, memory_id)? else {
        return Ok(None);
    };
    let values = embed::from_vec_blob(&blob, dim)?;
    Ok(Some(SyncVector {
        model,
        dims: dim as u32,
        f32: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            embed::to_vec_blob(&values),
        ),
    }))
}
