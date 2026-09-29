//! `project show` / `GET /api/v1/projects/{id}`: one charter by id, ported
//! from the platform's `getProject`. A malformed id is the platform's
//! `z.uuid()` refusal (`400`); an unknown one `404 project_not_found`.

use serde::{Deserialize, Serialize};

use crate::domains::projects::authority::{Actor, Command, Verb, sealed};
use crate::domains::projects::binding::{self, TransferView};
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
    /// Its transfer binding (#342), when it was transferred; absent otherwise,
    /// so an unbound project answers the platform's shape unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer: Option<TransferView>,
}

impl sealed::Sealed for Request {}

impl Command for Request {
    type Response = Response;

    fn verb(&self) -> Verb {
        Verb::ProjectShow
    }

    /// Read the project `self.id` names.
    fn execute(self, ctx: &mut Ctx<'_>, _actor: &Actor) -> Result<Response> {
        let id = uuid::canonical(&self.id)
            .ok_or_else(|| Error::from(ProjectError::invalid_field("projectId", "invalid")))?;
        let conn = ctx.conn()?;
        let row = project_read::project(conn, &id)?.ok_or_else(|| {
            Error::from(ProjectError::ProjectNotFound {
                project_id: self.id.clone(),
            })
        })?;
        let project = view::load(conn, vec![row])?.pop().ok_or_else(|| {
            Error::from(ProjectError::ProjectNotFound {
                project_id: self.id,
            })
        })?;
        let transfer = binding::find(conn, &id)?;
        Ok(Response { project, transfer })
    }
}

#[cfg(test)]
#[path = "tests/show.rs"]
mod tests;
