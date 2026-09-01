//! `api::refresh_refs` — `POST /api/v1/memories/{id}/references/refresh`:
//! re-pin every anchored code reference to the current HEAD (console-api
//! spec §4).
//!
//! A `references` entry captures the code state at save time (git blob OID +
//! commit + branch); `retrieval::code_ref_status` later compares that anchor
//! against the live repo to report `fresh|stale|ghost|unpinned|unknown`. This
//! surface moves the anchor forward: for every ref whose repo root resolves
//! (`repo_marker.root_path`, via `serve::repo_root::resolve_root`) it
//! re-reads the HEAD-tree blob, HEAD commit and branch, rewrites the
//! frontmatter in place, and re-mirrors — so a `stale` ref the user has
//! reviewed becomes `fresh` again without re-saving the memory under a new
//! id.
//!
//! Two deliberate degradations, neither an error: a repo whose root cannot
//! be resolved is skipped and named in `skipped`, and a file missing from the
//! HEAD tree keeps its old blob (so it still classifies `ghost` rather than
//! silently becoming `unpinned`).

use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::Connection;
use serde::Serialize;

use crate::api::Ctx;
use crate::api::show::CodeRefRow;
use crate::git_utils;
use crate::memory::{MemoryStore, Ref};
use crate::prelude::*;
use crate::serve::repo_root::{RootOverrides, resolve_root};

/// `POST /api/v1/memories/{id}/references/refresh` response.
#[derive(Serialize, Debug)]
pub struct Response {
    /// The memory whose references were re-pinned.
    pub id: String,
    /// How many references were re-anchored — every ref whose repo root
    /// resolved, including one whose file has since left the HEAD tree
    /// (its commit/branch move forward, its blob is kept).
    pub refreshed: usize,
    /// Reference ids left untouched because their repo root could not be
    /// resolved (never indexed, or the working tree is gone).
    pub skipped: Vec<String>,
    /// The memory's code references after the refresh, re-classified — the
    /// same rows `GET /api/v1/memories/{id}` reports.
    pub code_refs: Vec<CodeRefRow>,
}

/// Per-repo working-tree roots, resolved once each. `None` records a repo
/// whose root did not resolve so a second ref into it does not re-query.
type RootCache = HashMap<String, Option<PathBuf>>;

/// Re-pin memory `id`'s references. `Error::NotFound` for an unknown or
/// soft-deleted id (its markdown is in `.trash/`, which `MemoryStore::load`
/// does not scan).
pub fn run(ctx: &mut Ctx<'_>, id: &str) -> Result<Response> {
    let store = MemoryStore::new(ctx.paths.clone());
    let mut record = store.load(id)?;
    let id = record.frontmatter.id.clone();

    let mut roots = RootCache::new();
    let mut skipped = Vec::new();
    let refreshed = {
        let conn = ctx.conn()?;
        let refs = &mut record.frontmatter.references;
        repin_all(conn, &mut refs.files, false, &mut roots, &mut skipped)?
            + repin_all(conn, &mut refs.symbols, true, &mut roots, &mut skipped)?
    };
    if refreshed > 0 {
        store.rewrite(&record)?;
        // Re-mirroring re-materializes `code_ref` from the frontmatter, so
        // the freshness read below sees the new anchors.
        crate::api::update::mirror_record(ctx, &record)?;
    }

    let shown = crate::api::show::run(ctx, crate::api::show::Request { id: id.clone() })?;
    Ok(Response {
        id,
        refreshed,
        skipped,
        code_refs: shown.code_refs,
    })
}

/// Re-anchor every ref in `refs`, returning how many were re-anchored and
/// pushing the ids of the rest onto `skipped`.
fn repin_all(
    conn: &Connection,
    refs: &mut [Ref],
    is_symbol: bool,
    roots: &mut RootCache,
    skipped: &mut Vec<String>,
) -> Result<usize> {
    let mut refreshed = 0;
    for r in refs {
        let Some((repo, path)) = split_ref(&r.id, is_symbol) else {
            skipped.push(r.id.clone());
            continue;
        };
        let Some(root) = root_for(conn, repo, roots) else {
            skipped.push(r.id.clone());
            continue;
        };
        // A file gone from the HEAD tree keeps its captured blob so the ref
        // still classifies `ghost` (a cleared blob would read as `unpinned`,
        // which is a different — and wrong — story).
        if let Some(blob) = git_utils::blob_oid_at_head(&root, path)? {
            r.blob = Some(blob);
        }
        r.commit = Some(git_utils::current_head(&root)?);
        r.branch = git_utils::current_branch(&root)?;
        refreshed += 1;
    }
    Ok(refreshed)
}

/// The working-tree root for `repo`, resolved through the same
/// `repo_marker.root_path` lookup `retrieval::code_ref_fetch` classifies
/// against, and cached per repo. `None` when the repo has no usable root —
/// the caller degrades that ref to `skipped`.
fn root_for(conn: &Connection, repo: &str, roots: &mut RootCache) -> Option<PathBuf> {
    roots
        .entry(repo.to_string())
        .or_insert_with(|| resolve_root(conn, repo, &RootOverrides::new()).ok())
        .clone()
}

/// Split a reference id into `(repo, repo-relative path)`. File ids are
/// `<repo>:<path>`, symbol ids `<repo>:<path>:<symbol>` — the same
/// `splitn`-based parse `retrieval::code_ref_collect` uses, so a path
/// containing a colon is treated identically on both sides. `None` for a
/// malformed id (missing or empty segment).
fn split_ref(id: &str, is_symbol: bool) -> Option<(&str, &str)> {
    let mut parts = id.splitn(if is_symbol { 3 } else { 2 }, ':');
    let repo = parts.next()?;
    let path = parts.next()?;
    if is_symbol && parts.next().is_none_or(str::is_empty) {
        return None;
    }
    (!repo.is_empty() && !path.is_empty()).then_some((repo, path))
}

#[cfg(test)]
#[path = "tests/refresh_refs.rs"]
mod tests;
