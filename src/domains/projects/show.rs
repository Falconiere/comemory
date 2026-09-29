//! `project show` / `GET /api/v1/projects/{id}`: one charter by id, ported
//! from the platform's `getProject`. A malformed id is the platform's
//! `z.uuid()` refusal (`400`); an unknown one `404 project_not_found`.

use serde::{Deserialize, Serialize};

use crate::domains::projects::view::{self, ProjectView};
use crate::prelude::*;
use crate::store::project_read;
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;
use crate::utilities::uuid;

/// `project show` / `GET /api/v1/projects/{id}` request.
#[derive(Deserialize, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// The project's UUID.
    pub id: String,
}

/// One project.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Its charter view.
    pub project: ProjectView,
}

/// Read the project `req.id` names.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let id = uuid::canonical(&req.id)
        .ok_or_else(|| Error::from(ProjectError::invalid_field("projectId", "invalid")))?;
    let conn = ctx.conn()?;
    let row = project_read::project(conn, &id)?.ok_or_else(|| {
        Error::from(ProjectError::ProjectNotFound {
            project_id: req.id.clone(),
        })
    })?;
    let project = view::load(conn, vec![row])?
        .pop()
        .ok_or_else(|| Error::from(ProjectError::ProjectNotFound { project_id: req.id }))?;
    Ok(Response { project })
}

#[cfg(test)]
#[path = "tests/show.rs"]
mod tests;
