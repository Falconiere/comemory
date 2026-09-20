//! Reading the model back: the newest live memory tagged
//! [`TAG`](super::model::TAG) for a repo, and the fenced JSON inside its body.
//!
//! Two machines can each supersede the same predecessor and push, so more than
//! one live architecture memory can exist for one repo after a sync. Newest
//! wins here, in the console, and in `check`; the next save supersedes
//! whichever this returned.

use crate::domains::architecture::model::{Model, TAG};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::memory_list::{ListFilter, SortBy, list_memories};

/// The architecture memory currently in force for a repo.
#[derive(Debug, Clone)]
pub struct Current {
    /// 8-hex memory id.
    pub id: String,
    /// RFC 3339 creation timestamp of that memory.
    pub created: String,
    /// The parsed model.
    pub model: Model,
}

/// The newest live architecture memory for `repo`, or `None`.
pub fn find(conn: &Connection, repo: &str) -> Result<Option<Current>> {
    let filter = ListFilter {
        repo: Some(repo),
        tag: Some(TAG),
        ..ListFilter::default()
    };
    let page = list_memories(conn, &filter, 1, 0, SortBy::Created)?;
    let Some(row) = page.rows.into_iter().next() else {
        return Ok(None);
    };
    Ok(Some(Current {
        model: parse_body(&row.body)?,
        id: row.id,
        created: row.created,
    }))
}

/// [`find`], failing with [`Error::NotFound`] when no model has been saved.
pub fn require(conn: &Connection, repo: &str) -> Result<Current> {
    find(conn, repo)?.ok_or_else(|| {
        Error::NotFound(format!(
            "no architecture model saved for repo {repo:?}; run `comemory architecture scaffold \
             --repo {repo} | comemory architecture save - --repo {repo}`"
        ))
    })
}

/// Parse the model out of a memory body: the first ```json fence, falling back
/// to the whole body when it is bare JSON.
pub fn parse_body(body: &str) -> Result<Model> {
    let json = fenced_json(body).unwrap_or(body);
    Ok(serde_json::from_str(json)?)
}

/// The contents of the first ```json fence in `body`, if there is one.
fn fenced_json(body: &str) -> Option<&str> {
    let open = body.find("```json")?;
    let after = open + "```json".len();
    let rest = &body[after..];
    let start = rest.find('\n')? + 1;
    let end = rest[start..].find("```")? + start;
    Some(&rest[start..end])
}

#[cfg(test)]
#[path = "tests/current.rs"]
mod tests;
