//! Import write path — markdown + SQLite mirror + sync log (rules 8–10).

use crate::config::Config;
use crate::domains::memories::frontmatter::Frontmatter;
use crate::domains::memories::{MemoryRecord, MemoryStore, SaveParams, journal, mirror};
use crate::domains::sync::exchange::{ImportEntry, ImportItemResult, ImportStatus, SyncRecord};
use crate::domains::sync::vector_rule;
use crate::prelude::*;
use crate::store::replica_journal::{ReplicaOp, ReplicaOrigin};
use crate::store::{Connection, simhash_scan};
use crate::utilities::context::Ctx;

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
    // The same rule the replica wire applies: an unusable vector never
    // refuses the memory, it lands in the needs-embedding backlog instead.
    let verdict = vector_rule::decide(conn, record.vector.as_ref())?;
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
    mirror::insert_row(
        &tx,
        &rec.frontmatter,
        &rec.body,
        rec.slug.as_str(),
        &md_path,
        &rec.frontmatter.tags,
    )?;
    vector_rule::apply(&tx, &rec.frontmatter.id, &verdict, &entry.at)?;
    let seq = journal::record_write(
        &tx,
        ReplicaOp::Upsert,
        &rec.frontmatter,
        &rec.body,
        &entry.at,
        ReplicaOrigin::Sync,
        None,
    )?
    .legacy_seq;
    tx.commit()?;
    let _stale = crate::domains::graph::derived::refresh_derived_best_effort(conn);
    out.status = ImportStatus::Accepted;
    out.seq = Some(seq);
    out.duplicate_of = duplicate_of;
    Ok(())
}

/// Rule 8/9 — apply frontmatter from the wire onto a live markdown file.
///
/// Returns the patched record so the caller can journal it without reading
/// the file back.
pub(crate) fn patch_frontmatter(
    ctx: &mut Ctx<'_>,
    entry: &ImportEntry,
    record: &SyncRecord,
    author_override: Option<&str>,
) -> Result<MemoryRecord> {
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
    crate::domains::memories::update::mirror_record(ctx, &rec)?;
    Ok(rec)
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

fn near_duplicate(conn: &Connection, body: &str, self_id: &str, radius: u32) -> Option<String> {
    let hash = crate::utilities::simhash::of_body(body);
    simhash_scan::live_simhashes(conn, None, Some(self_id))
        .ok()?
        .into_iter()
        .map(|row| {
            (
                row.id,
                crate::utilities::simhash::hamming64(hash, row.simhash as u64),
            )
        })
        .filter(|(_, d)| *d <= radius)
        .min_by_key(|(_, d)| *d)
        .map(|(id, _)| id)
}
