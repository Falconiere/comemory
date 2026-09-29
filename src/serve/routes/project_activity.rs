//! `GET /api/v1/projects/{id}/activity` (#331): one project's activity page,
//! the `domains::projects::activity_page` core under the caller's envelope
//! through `projects`' query-string read. The path names the project and the
//! query the page; the route-table row lives with its siblings in
//! [`super::projects::table_entries`].

use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::response::Response;

use crate::domains::projects::activity_page::Request;
use crate::serve::AppState;
use crate::serve::routes::projects::{parsed, read};

/// `project.activity`'s route command, shared by the table and the handler.
pub const ACTIVITY: &str = "project.activity";

/// `GET /api/v1/projects/{id}/activity` — a keyset page of the project's
/// events, parsed before any store access.
pub async fn page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    raw: std::result::Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Response {
    let req = parsed(raw, field).map(|page| Request { id, ..page });
    read(state, ACTIVITY, req).await
}

/// One query pair stored on `req`; `None` refuses an unknown key (the id
/// comes from the path) or a non-numeric `limit`. The core judges the
/// cursor's and the order's values.
pub fn field(req: &mut Request, key: &str, value: String) -> Option<()> {
    match key {
        "limit" => req.limit = Some(value.parse().ok()?),
        "cursor" => req.cursor = Some(value),
        "order" => req.order = Some(value),
        _ => return None,
    }
    Some(())
}

#[cfg(test)]
#[path = "tests/project_activity.rs"]
mod tests;
