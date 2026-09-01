//! `api::update` — `PATCH /api/v1/memories/{id}`: frontmatter-only patch in
//! place, body patch as a superseding re-save (console-api spec §4).
//!
//! The split is forced by the data model, not chosen: a memory id is the
//! 8-hex prefix of `SHA-256(body.trim_end())`, so metadata can be edited
//! under the same id but content cannot. A patch that changes the body is
//! therefore a NEW memory that `supersedes` the old one — comemory's native
//! edit — and the response reports both ids.
//!
//! **Reference caveat.** The re-save path re-supplies the old frontmatter's
//! `references` as `ref_file`/`ref_symbol` values, which `api::save`
//! re-anchors against the *calling process's* cwd repo (its documented cwd
//! semantics). Over HTTP that is the server's cwd, so a ref pinned at the
//! original save may come back unpinned on the new id;
//! `POST /memories/{id}/references/refresh` re-pins it against the repo's
//! recorded root.

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::memory::{Frontmatter, Kind, MemoryRecord, MemoryStore, id};
use crate::prelude::*;
use crate::store::memory_row;

/// `PATCH /api/v1/memories/{id}` request. Every field is optional: an absent
/// field is left untouched, and an empty object is a no-op patch that still
/// answers `200` with an empty `changed`.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// New memory kind.
    pub kind: Option<Kind>,
    /// New owning repo.
    pub repo: Option<String>,
    /// Replacement tag list (de-duplicated, order preserved).
    pub tags: Option<Vec<String>>,
    /// New quality rating, validated `1..=5`.
    pub quality: Option<u8>,
    /// New body. Changing it mints a new id — see the module doc.
    pub body: Option<String>,
    /// New title. Folded into the body as its first line, exactly as
    /// `api::save::Request::title` is, so patching a title is a body patch.
    pub title: Option<String>,
}

/// `PATCH /api/v1/memories/{id}` response.
#[derive(Serialize, Debug)]
pub struct Response {
    /// The memory's id after the patch: unchanged for a frontmatter-only
    /// patch, the newly minted content id when the body changed.
    pub id: String,
    /// On-disk path of the markdown file behind [`Response::id`].
    pub path: String,
    /// The old id, present only when the body changed and a new memory was
    /// minted to supersede it.
    pub superseded: Option<String>,
    /// Names of the fields this patch actually changed (`kind`, `repo`,
    /// `tags`, `quality`, `body`). Empty for a no-op patch.
    pub changed: Vec<&'static str>,
}

/// Apply a patch to memory `id`. Unknown or soft-deleted ids are
/// `Error::NotFound` (a soft-deleted memory's markdown lives in `.trash/`,
/// which `MemoryStore::load` does not scan); an out-of-range `quality` is
/// `Error::BadRequest` before anything is touched.
pub fn run(ctx: &mut Ctx<'_>, id: &str, req: Request) -> Result<Response> {
    if let Some(quality) = req.quality {
        validate_quality(quality)?;
    }
    let store = MemoryStore::new(ctx.paths.clone());
    let mut record = store.load(id)?;
    match patched_body(&req, &record) {
        Some(body) => resave(ctx, &record, &req, body),
        None => patch_in_place(ctx, &store, &mut record, &req),
    }
}

/// `1..=5`, mirroring `api::save::run`'s check (HTTP has no clap validator).
fn validate_quality(quality: u8) -> Result<()> {
    if (1..=5).contains(&quality) {
        Ok(())
    } else {
        Err(Error::BadRequest(format!(
            "quality must be in 1..=5, got {quality}"
        )))
    }
}

/// The body this patch would write, or `None` when the content is unchanged.
///
/// `None` covers both "no `body`/`title` field at all" and "a `body`/`title`
/// that folds back to the same bytes" — the latter matters because the id is
/// the content hash, so a re-save would try to supersede itself.
fn patched_body(req: &Request, record: &MemoryRecord) -> Option<String> {
    if req.body.is_none() && req.title.is_none() {
        return None;
    }
    let base = req.body.as_deref().unwrap_or(&record.body);
    let folded = fold_title(req.title.as_deref(), base);
    (id::memory_id(&folded) != record.frontmatter.id).then_some(folded)
}

/// Fold `title` into `body` as its first line, mirroring
/// `api::save::run`'s title rule so this module can tell — *before* calling
/// save — whether the patch changes the content hash. The folded body is
/// then handed to save with `title: None`, so the rule is applied exactly
/// once per patch.
fn fold_title(title: Option<&str>, body: &str) -> String {
    let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) else {
        return body.to_string();
    };
    if body.trim_start().starts_with(title) {
        return body.to_string();
    }
    format!("{title}\n\n{}", body.trim_start())
}

/// Body patch: save `body` as a new memory carrying the old frontmatter's
/// metadata (overridden by any patched field) and `supersedes: [old id]`.
/// The old memory stays on disk and in the mirror, demoted by the supersede
/// edge exactly as a hand-run `comemory save --supersedes` would leave it.
fn resave(ctx: &mut Ctx<'_>, old: &MemoryRecord, req: &Request, body: String) -> Result<Response> {
    let fm = &old.frontmatter;
    let changed = changed_fields(req, fm, true);
    let save_req = crate::api::save::Request {
        body,
        // Already folded by `patched_body`; folding again would be a no-op
        // but re-derives the rule at a second site.
        title: None,
        kind: req.kind.unwrap_or(fm.kind),
        repo: req.repo.clone().unwrap_or_else(|| fm.repo.clone()),
        tags: dedup(req.tags.as_deref().unwrap_or(&fm.tags)),
        author: fm.author.clone(),
        quality: req.quality.unwrap_or(fm.quality),
        supersedes: vec![fm.id.clone()],
        vector: None,
        ref_file: fm.references.files.iter().map(|r| r.id.clone()).collect(),
        ref_symbol: fm.references.symbols.iter().map(|r| r.id.clone()).collect(),
    };
    let saved = crate::api::save::run(ctx, save_req, false, None)?;
    Ok(Response {
        id: saved.id,
        path: saved.path,
        superseded: Some(fm.id.clone()),
        changed,
    })
}

/// Frontmatter-only patch: rewrite the markdown in place under the same id
/// (the slug derives from the body, so the filename is unchanged), then
/// re-mirror. A patch that changes nothing skips both writes and still
/// answers `200` with an empty `changed`.
fn patch_in_place(
    ctx: &mut Ctx<'_>,
    store: &MemoryStore,
    record: &mut MemoryRecord,
    req: &Request,
) -> Result<Response> {
    let changed = changed_fields(req, &record.frontmatter, false);
    let path = record.path.to_string_lossy().into_owned();
    let id = record.frontmatter.id.clone();
    if changed.is_empty() {
        return Ok(Response {
            id,
            path,
            superseded: None,
            changed,
        });
    }
    apply(&mut record.frontmatter, req);
    store.rewrite(record)?;
    mirror_record(ctx, record)?;
    Ok(Response {
        id,
        path,
        superseded: None,
        changed,
    })
}

/// Overwrite the patched frontmatter fields. `id`, `created`,
/// `content_hash`, `references` and `relations` are deliberately untouched:
/// the first three describe the body (unchanged on this path) and the last
/// two are owned by save / the reference-refresh surface.
fn apply(fm: &mut Frontmatter, req: &Request) {
    if let Some(kind) = req.kind {
        fm.kind = kind;
    }
    if let Some(repo) = &req.repo {
        fm.repo.clone_from(repo);
    }
    if let Some(tags) = &req.tags {
        fm.tags = dedup(tags);
    }
    if let Some(quality) = req.quality {
        fm.quality = quality;
    }
}

/// Which supplied fields differ from what the memory already carries.
/// A field set to its current value is not a change, so a re-PATCH of the
/// same payload reports `changed: []` and rewrites nothing.
fn changed_fields(req: &Request, fm: &Frontmatter, body_changed: bool) -> Vec<&'static str> {
    let mut out = Vec::new();
    if req.kind.is_some_and(|k| k != fm.kind) {
        out.push("kind");
    }
    if req.repo.as_ref().is_some_and(|r| *r != fm.repo) {
        out.push("repo");
    }
    if req.tags.as_deref().is_some_and(|t| dedup(t) != fm.tags) {
        out.push("tags");
    }
    if req.quality.is_some_and(|q| q != fm.quality) {
        out.push("quality");
    }
    if body_changed {
        out.push("body");
    }
    out
}

/// Drop empty and repeated tags, preserving first-seen order — the same
/// normalization `store::memory_row::insert` applies defense-in-depth, done
/// here too so the frontmatter written to disk matches the mirrored rows.
fn dedup(tags: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    tags.iter()
        .filter(|t| !t.is_empty() && seen.insert(t.as_str()))
        .cloned()
        .collect()
}

/// Re-mirror an edited record into `comemory.db` in one transaction and
/// refresh the derived artifacts, the SQLite half of every in-place markdown
/// edit. `memory_row::insert` rewrites tags, FTS, outgoing edges and the
/// `code_ref` anchors for the id and clears `deleted_at`, so this is also
/// exactly what a restore needs — hence `pub(crate)`, shared with
/// [`crate::api::restore`] and [`crate::api::refresh_refs`] rather than
/// re-derived in each.
pub(crate) fn mirror_record(ctx: &mut Ctx<'_>, record: &MemoryRecord) -> Result<()> {
    let conn = ctx.conn()?;
    let fm = &record.frontmatter;
    let md_path = record.path.to_string_lossy().into_owned();
    let tx = conn.transaction()?;
    memory_row::insert(
        &tx,
        fm,
        &record.body,
        record.slug.as_str(),
        &md_path,
        &fm.tags,
    )?;
    tx.commit()?;
    crate::graph::derived::refresh_derived_best_effort(conn);
    Ok(())
}

#[cfg(test)]
#[path = "tests/update.rs"]
mod tests;
