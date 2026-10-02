//! Turn a raw project query string into its core request, answering every
//! malformed pair with the platform's schema-edge `invalid_request` (`400`)
//! naming the key. Bodies are [`crate::utilities::project_body`].

use std::collections::HashSet;

use crate::domains::projects::list;
use crate::prelude::*;
use crate::utilities::project_error::ProjectError;

/// `GET /projects`'s pair `key=value` stored on `req`; `None` refuses an
/// unknown key, a non-numeric `limit` or an `includeArchived` other than
/// `true`/`false`.
pub fn list_field(req: &mut list::Request, key: &str, value: String) -> Option<()> {
    match key {
        "limit" => req.limit = Some(value.parse().ok()?),
        "cursor" => req.cursor = Some(value),
        "status" => req.status = Some(value),
        "health" => req.health = Some(value),
        "includeArchived" => req.include_archived = Some(value.parse().ok()?),
        _ => return None,
    }
    Some(())
}

/// A route's query pairs as its core request `R`, each stored by `field`. A
/// malformed query string, a repeated key or a pair `field` refuses is the
/// schema-edge `400` naming the key; `workspaceId` is accepted and ignored.
pub fn query<R: Default, E>(
    pairs: std::result::Result<Vec<(String, String)>, E>,
    field: fn(&mut R, &str, String) -> Option<()>,
) -> Result<R> {
    let pairs = pairs.map_err(|_| Error::from(ProjectError::invalid_field("query", "invalid")))?;
    let mut req = R::default();
    let mut seen = HashSet::new();
    for (key, value) in pairs {
        let accepted = seen.insert(key.clone())
            && (key == "workspaceId" || field(&mut req, &key, value).is_some());
        if !accepted {
            return Err(ProjectError::invalid_field(&key, "invalid").into());
        }
    }
    Ok(req)
}

#[cfg(test)]
#[path = "tests/project_request.rs"]
mod tests;
