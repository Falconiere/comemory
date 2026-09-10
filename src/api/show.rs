//! `api::show::{Request, run}` — the shared middle of `comemory show` /
//! `GET /api/v1/memories/{id}`: one memory in full — body, frontmatter,
//! activation, and code-reference freshness — in one round trip.
//!
//! The first seven [`Response`] fields (`id`, `kind`, `repo`, `slug`, `tags`,
//! `references`, `path`) are exactly today's `serve::routes::memories`
//! `MemoryDetail` shape, produced the same way — [`memory_meta::fetch_meta`]
//! plus [`abs_path`] — so `GET /api/v1/memories/{id}` now answers through
//! this module instead of duplicating the lookup (see the rewired handler in
//! `src/serve/routes/memories.rs`). Everything after `path` is additive.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::api::Ctx;
use crate::memory::References;
use crate::output::search::{abs_path, title_of};
use crate::prelude::*;
use crate::retrieval::code_ref_collect;
use crate::retrieval::code_ref_fetch::RefStatusCache;
use crate::retrieval::rerank::live_superseder;
use crate::retrieval::score;
use crate::store::{Connection, edges_retrieval, memory_meta};

/// `comemory show` / `GET /api/v1/memories/{id}` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// 8-hex memory id to show in full.
    pub id: String,
}

/// One code reference harvested from the body, freshness-classified against
/// the live repo state via [`crate::retrieval::code_ref_status`].
#[derive(Serialize, Debug)]
pub struct CodeRefRow {
    /// Repo-relative path of the referenced file.
    pub path: String,
    /// Qualified address `<repo>:<path>[:<symbol>]` this reference resolves.
    pub anchor: String,
    /// The git blob OID the reference was pinned at, when it was pinned
    /// (`None` for a bare backtick mention with no version anchor).
    pub blob: Option<String>,
    /// Freshness verdict (`fresh|stale|ghost|unpinned|unknown`).
    pub status: String,
}

/// One memory in full, as emitted under `--json` and the
/// `/api/v1/memories/{id}` `data` field. Field order matters: the first
/// seven are the pre-existing `MemoryDetail` contract (see the module doc)
/// and must not be reordered, renamed, or retyped. Fields after `path` are
/// additive — including `author`, which serializes as a stable empty
/// string when unset (never `null`), matching `list::Row`.
#[derive(Serialize, Debug)]
pub struct Response {
    /// 8-hex memory id.
    pub id: String,
    /// Canonical lowercase kind string.
    pub kind: String,
    /// Owning repo, or `None` when the memory has none.
    pub repo: Option<String>,
    /// On-disk file stem `{id}-{slug}`.
    pub slug: String,
    /// Tag list from `memory_tags`.
    pub tags: Vec<String>,
    /// Code references harvested from the body.
    pub references: References,
    /// Absolute path to the memory's markdown file.
    pub path: String,
    /// Frontmatter author, or empty string when unset. Stable string type
    /// (never `null`) — same contract as [`crate::api::list::Row::author`].
    pub author: String,
    /// First non-empty trimmed line of the body.
    pub title: String,
    /// Full memory body, verbatim.
    pub body: String,
    /// Frontmatter quality (1..=5).
    pub quality: u8,
    /// RFC 3339 creation timestamp.
    pub created: String,
    /// RFC 3339 last-update timestamp.
    pub updated: String,
    /// Total number of times this memory has been returned by a tracked
    /// `search` / `context` run.
    pub access_count: u64,
    /// RFC 3339 timestamp of the most recent tracked access; `None` when
    /// the memory has never been accessed since it was saved.
    pub last_accessed: Option<String>,
    /// ACT-R activation (`retrieval::score::activation`) computed from
    /// `access_count` and days since `last_accessed` (falling back to
    /// `created` when never accessed), under the configured decay.
    pub activation: f64,
    /// Memory-graph PageRank score (`memories.rank_score`).
    pub rank_score: f64,
    /// Live memory id that supersedes this one, if any — the same
    /// `edges` join `retrieval::rerank`'s supersede penalty uses.
    pub superseded_by: Option<String>,
    /// Direct code references, freshness-classified.
    pub code_refs: Vec<CodeRefRow>,
}

/// Show one memory in full. `Error::NotFound` for an unknown or
/// soft-deleted id (`fetch_meta` and the extra-fields lookup both exclude
/// `deleted_at IS NOT NULL` rows). A data dir with no `comemory.db` yet
/// (nothing has ever been saved) short-circuits to the same `NotFound`
/// before opening a connection — the same must-not-create-the-db-as-a-
/// side-effect-of-a-read invariant `api::stats`/`api::gc` keep, applied to
/// a single-id lookup instead of a counter.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    if !ctx.paths.db_path().exists() {
        return Err(Error::NotFound(format!("memory not found: {}", req.id)));
    }
    let cfg = ctx.cfg;
    let data_dir = ctx.paths.data_dir();
    let conn = ctx.conn()?;

    let mut meta = memory_meta::fetch_meta(conn, &[req.id.as_str()])?;
    let entry = meta
        .remove(&req.id)
        .ok_or_else(|| Error::NotFound(format!("memory not found: {}", req.id)))?;
    let extra = memory_meta::fetch_extra(conn, &req.id)?
        .ok_or_else(|| Error::NotFound(format!("memory not found: {}", req.id)))?;

    let path = abs_path(Some(&entry), data_dir);
    let title = title_of(&extra.body);
    let anchor_ts = extra.last_accessed.as_deref().unwrap_or(&extra.created);
    let days = score::days_since(anchor_ts, OffsetDateTime::now_utc());
    let activation = score::activation(extra.access_count, days, cfg.rank.decay);
    let superseded_by = live_superseder(conn, &req.id, None)?;
    let code_refs = code_refs_for(conn, &req.id)?;

    Ok(Response {
        id: req.id,
        kind: entry.kind,
        repo: entry.repo,
        slug: entry.slug,
        tags: entry.tags,
        references: entry.references,
        path,
        author: extra.author,
        title,
        body: extra.body,
        quality: extra.quality,
        created: extra.created,
        updated: extra.updated,
        access_count: extra.access_count,
        last_accessed: extra.last_accessed,
        activation,
        rank_score: extra.rank_score,
        superseded_by,
        code_refs,
    })
}

/// Freshness-classify every direct `references_file` / `references_symbol`
/// edge from `id`. Reuses [`code_ref_collect`] to resolve each edge into a
/// `RawRef` and [`RefStatusCache`] to classify it — the same building
/// blocks `retrieval::bundle` walks multi-hop for `comemory context`,
/// restricted here to this memory's own direct references (depth 1).
///
/// `cross_link::extract_refs` mines a single `` `repo:path:symbol` `` body
/// token into BOTH a `references_file` and a `references_symbol` edge (the
/// symbol implies its file); [`drop_implied_file_refs`] collapses that pair
/// back to the one symbol-level citation the user actually wrote, so a
/// memory citing one symbol reports one `code_refs` row (spec AC-6), not two.
fn code_refs_for(conn: &Connection, id: &str) -> Result<Vec<CodeRefRow>> {
    let anchors = code_ref_collect::anchor_map(conn, id)?;
    let edges = edges_retrieval::direct_reference_edges(conn, id)?;

    let mut resolved = Vec::with_capacity(edges.len());
    for (rel, dst_id) in edges {
        if let Some(raw) = code_ref_collect::ref_from_edge(conn, &rel, &dst_id, &anchors)? {
            resolved.push(raw);
        }
    }
    drop_implied_file_refs(&mut resolved);

    let mut cache = RefStatusCache::default();
    let mut out = Vec::with_capacity(resolved.len());
    for raw in resolved {
        let status = cache.status(
            conn,
            &raw.repo,
            &raw.path,
            raw.is_symbol,
            raw.pinned_blob.as_deref(),
            raw.symbol_id.is_some(),
        );
        out.push(CodeRefRow {
            path: raw.path,
            anchor: raw.id,
            blob: raw.pinned_blob,
            status: status.as_str().to_string(),
        });
    }
    Ok(out)
}

/// Drop a plain file ref whenever a symbol ref for the same `(repo, path)`
/// is also present — the symbol ref is strictly more specific and both were
/// mined from the same body token. See [`code_refs_for`] for why this
/// matters.
fn drop_implied_file_refs(raws: &mut Vec<code_ref_collect::RawRef>) {
    let symbol_files: std::collections::BTreeSet<(String, String)> = raws
        .iter()
        .filter(|r| r.is_symbol)
        .map(|r| (r.repo.clone(), r.path.clone()))
        .collect();
    raws.retain(|r| r.is_symbol || !symbol_files.contains(&(r.repo.clone(), r.path.clone())));
}

#[cfg(test)]
#[path = "tests/show.rs"]
mod tests;
