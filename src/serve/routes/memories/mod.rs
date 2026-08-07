//! `GET /api/v1/memories` (`api::list`) and `GET /api/v1/memories/{id}`
//! (new: a trivial single-row lookup via `store::memory_meta::fetch_meta`).
//! `GET|POST /api/v1/memories/search` and `GET|POST /api/v1/context` live
//! in [`search`]; the mutating routes (`POST /memories`, `DELETE
//! /memories/{id}`, `POST /feedback`) live in [`write`] — both merged into
//! this resource's [`router`], `write`'s own route-table entries appended
//! at [`super::table`] alongside this module's (the `maint`/`prune`
//! sibling-module pattern).

use std::time::Instant;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::get;
use serde::Serialize;

use crate::api::{self, Ctx};
use crate::memory::References;
use crate::output::search::abs_path;
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond, run_blocking};
use crate::store::memory_meta;

/// `GET|POST /memories/search` (`api::search`) and `GET|POST /context`
/// (`api::context`).
pub mod search;
/// `POST /memories`, `DELETE /memories/{id}`, `POST /feedback` — the
/// mutating routes (`api::{save,delete,feedback}`).
pub mod write;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/memories",
            command: "list",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/memories/{id}",
            command: "memories.get",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/memories/search",
            command: "search",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/memories/search",
            command: "search",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/context",
            command: "context",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/context",
            command: "context",
            mutating: false,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/memories", get(list))
        .route("/api/v1/memories/{id}", get(get_one))
        .merge(search::router(state.clone()))
        .merge(write::router(state))
}

/// `GET /api/v1/memories` — page live memories (`api::list`).
async fn list(State(state): State<AppState>, Query(req): Query<api::list::Request>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::list::run(&mut ctx, req)
    })
    .await;
    respond("list", result, started)
}

/// One memory's navigation metadata, the `GET /api/v1/memories/{id}` body.
#[derive(Serialize)]
struct MemoryDetail {
    /// 8-hex memory id.
    id: String,
    /// Canonical lowercase kind string.
    kind: String,
    /// Owning repo, or `None` when the memory has none.
    repo: Option<String>,
    /// On-disk file stem `{id}-{slug}`.
    slug: String,
    /// Tag list from `memory_tags`.
    tags: Vec<String>,
    /// Code references harvested from the body.
    references: References,
    /// Absolute path to the memory's markdown file.
    path: String,
}

/// `GET /api/v1/memories/{id}` — single-row lookup. `404 not_found` when
/// the id is absent or soft-deleted (`fetch_meta` already excludes
/// `deleted_at IS NOT NULL` rows).
async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let conn = state.conn()?;
        let mut meta = memory_meta::fetch_meta(&conn, &[id.as_str()])?;
        let entry = meta
            .remove(&id)
            .ok_or_else(|| Error::NotFound(format!("memory not found: {id}")))?;
        let path = abs_path(Some(&entry), state.paths().data_dir());
        Ok(MemoryDetail {
            id,
            kind: entry.kind,
            repo: entry.repo,
            slug: entry.slug,
            tags: entry.tags,
            references: entry.references,
            path,
        })
    })
    .await;
    respond("memories.get", result, started)
}
