//! Contained selected-folder embedding jobs (`POST /run/embedding`).

use crate::domains::documents::embedding;
use crate::prelude::*;
use crate::serve::envelope::Envelope;
use crate::serve::routes::mutate_response::with_lazy_ctx;
use crate::serve::routes::{RouteEntry, accepted, guard_job};
use crate::serve::{AppState, jobs};
use crate::utilities::{blocking::run_blocking, path_containment};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use std::path::Path;
use std::time::Instant;

/// Route metadata for command inventory and read-only policy.
pub fn table_entries() -> &'static [RouteEntry] {
    &[RouteEntry {
        method: "POST",
        path: "/run/embedding",
        command: "run",
        mutating: true,
    }]
}

/// Mount the explicit embedding job endpoint.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/run/embedding", post(run))
}

async fn run(State(state): State<AppState>, Json(mut req): Json<embedding::Request>) -> Response {
    let started = Instant::now();
    if let Err(response) = guard_job("run", &state) {
        return *response;
    }
    let containment_state = state.clone();
    let result = run_blocking(move || -> Result<embedding::Request> {
        let conn = containment_state.conn()?;
        let roots = containment_state.allowed_roots(&conn);
        drop(conn);
        let canonical = path_containment::contain_abs(&roots, Path::new(&req.path))?;
        req.path = canonical.to_string_lossy().into_owned();
        Ok(req)
    })
    .await;
    let req = match result {
        Ok(req) => req,
        Err(error) => return Envelope::err("run", &error, 0),
    };
    let job_state = state.clone();
    let job = jobs::spawn_job_with_id(
        state.jobs(),
        state.write_permit().clone(),
        "run",
        true,
        move |job_id| {
            let sink = jobs::worker::RegistryProgressSink::new(job_state.jobs().clone(), job_id);
            let output = with_lazy_ctx(&job_state, |ctx| {
                embedding::run_with_progress(ctx, req, Some(&sink))
            })?;
            serde_json::to_value(output).map_err(Error::Json)
        },
    );
    accepted("run", job, started)
}
