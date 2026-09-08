//! Import write path — markdown + SQLite mirror + sync log (rules 8–10).

use crate::api::Ctx;
use crate::api::sync::{
    ImportEntry, ImportItemResult, ImportStatus, SyncOp, SyncRecord, SyncVector,
};
use crate::config::Config;
use crate::memory::frontmatter::Frontmatter;
use crate::memory::{MemoryStore, SaveParams};
use crate::prelude::*;
use crate::store::{Connection, embed, memory_row, schema_meta, simhash_scan, sync_log, vector};

const MEMORY_DIM: usize = 1024;

/// Rule 10 — write a new memory from the wire record.
pub(crate) fn write_new_memory(
    ctx: &mut Ctx<'_>,
    entry: &ImportEntry,
    record: &SyncRecord,
    author_override: Option<&str>,
    cfg: &Config,
    out: &mut ImportItemResult,
) -> Result<()> {
    let paths = ctx.paths.clone();
    let conn = ctx.conn()?;
    let duplicate_of = near_duplicate(conn, &record.body, &entry.id, cfg.rank.near_dup_hamming);
    let vector = decode_vector(conn, record.vector.as_ref())?;
    let fm = frontmatter_from_wire(record, author_override);
    let params = SaveParams {
        body: &record.body,
        kind: fm.kind,
        repo: &fm.repo,
        tags: &fm.tags,
        author: &fm.author,
        quality: fm.quality,
        relations: fm.relations.clone(),
        references: fm.references.clone(),
        created: Some(fm.created),
    };
    let store = MemoryStore::new(paths);
    let rec = store.save(params)?;
    let md_path = rec.path.to_string_lossy();
    let tx = conn.transaction()?;
    memory_row::insert(
        &tx,
        &rec.frontmatter,
        &rec.body,
        rec.slug.as_str(),
        &md_path,
        &rec.frontmatter.tags,
    )?;
    if let Some(v) = vector.as_deref() {
        vector::replace_memory(&tx, &rec.frontmatter.id, v)?;
    }
    let seq = log_sync_upsert(&tx, entry)?;
    tx.commit()?;
    let _stale = crate::graph::derived::refresh_derived_best_effort(conn);
    out.status = ImportStatus::Accepted;
    out.seq = Some(seq);
    out.duplicate_of = duplicate_of;
    Ok(())
}

/// Rule 8/9 — apply frontmatter from the wire onto a live markdown file.
pub(crate) fn patch_frontmatter(
    ctx: &mut Ctx<'_>,
    entry: &ImportEntry,
    record: &SyncRecord,
    author_override: Option<&str>,
) -> Result<()> {
    let store = MemoryStore::new(ctx.paths.clone());
    let mut rec = store.load(&entry.id)?;
    let fm = frontmatter_from_wire(record, author_override);
    rec.frontmatter.kind = fm.kind;
    rec.frontmatter.repo = fm.repo;
    rec.frontmatter.tags = fm.tags;
    rec.frontmatter.quality = fm.quality;
    rec.frontmatter.references = fm.references;
    rec.frontmatter.relations = fm.relations;
    if let Some(author) = author_override {
        rec.frontmatter.author = author.to_string();
    }
    store.rewrite(&rec)?;
    crate::api::update::mirror_record(ctx, &rec)?;
    Ok(())
}

/// Append an upsert row to `sync_log` inside the caller's transaction.
pub(crate) fn log_sync_upsert(tx: &Connection, entry: &ImportEntry) -> Result<i64> {
    sync_log::append(
        tx,
        SyncOp::Upsert,
        &entry.id,
        &entry.content_hash,
        &entry.at,
        sync_log::SyncOrigin::Sync,
    )
}

fn frontmatter_from_wire(record: &SyncRecord, author_override: Option<&str>) -> Frontmatter {
    let wire = &record.frontmatter;
    Frontmatter {
        id: wire.id.clone(),
        kind: wire.kind,
        repo: wire.repo.clone(),
        tags: wire.tags.clone(),
        author: author_override.unwrap_or("").to_string(),
        created: wire.created,
        quality: wire.quality,
        schema: wire.schema,
        content_hash: wire.content_hash.clone(),
        references: wire.references.clone(),
        relations: wire.relations.clone(),
    }
}

fn decode_vector(conn: &Connection, wire: Option<&SyncVector>) -> Result<Option<Vec<f32>>> {
    let Some(wire) = wire else {
        return Ok(None);
    };
    let model = schema_meta::memory_vector_model(conn)?;
    if wire.model != model || wire.dims as usize != MEMORY_DIM {
        return Ok(None);
    }
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &wire.f32)
        .map_err(|e| Error::BadRequest(format!("vector base64: {e}")))?;
    let values = embed::from_vec_blob(&bytes, MEMORY_DIM)?;
    embed::guard_dim(&values, MEMORY_DIM)?;
    Ok(Some(values))
}

fn near_duplicate(conn: &Connection, body: &str, self_id: &str, radius: u32) -> Option<String> {
    let hash = crate::simhash::of_body(body);
    simhash_scan::live_simhashes(conn, None, Some(self_id))
        .ok()?
        .into_iter()
        .map(|row| (row.id, crate::simhash::hamming64(hash, row.simhash as u64)))
        .filter(|(_, d)| *d <= radius)
        .min_by_key(|(_, d)| *d)
        .map(|(id, _)| id)
}
