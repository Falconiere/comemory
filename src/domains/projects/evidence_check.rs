//! The checks an evidence attach (#346) runs before any store access, in the
//! platform's schema order: ids, the kind vocabulary, the text caps, the URL,
//! the repository and commit shapes, and the criterion list. A breach of a
//! cap answers `422` naming its limit; a value that does not parse `400`.

use serde_json::{Map, Value};

use crate::domains::projects::evidence;
use crate::domains::projects::evidence_add::Request;
use crate::domains::projects::limits;
use crate::domains::projects::list::vocabulary;
use crate::domains::sync::repository_identity::canonical_github_name;
use crate::prelude::*;
use crate::utilities::project_error::ProjectError;
use crate::utilities::uuid;

/// An attach request that passed every check that needs no store.
pub struct Valid {
    /// As the caller spelled it, for a `404`.
    pub raw_project_id: String,
    /// Canonical project UUID.
    pub project_id: String,
    /// As the caller spelled it, for a `404`.
    pub raw_work_item_id: Option<String>,
    /// Canonical work item UUID, when named.
    pub work_item_id: Option<String>,
    /// A member of the kind vocabulary.
    pub kind: String,
    /// The source system.
    pub source: String,
    /// The id in the source system.
    pub external_id: Option<String>,
    /// An absolute URL.
    pub url: Option<String>,
    /// Canonical lowercase `owner/name`.
    pub repo: Option<String>,
    /// Lowercase hex.
    pub commit_sha: Option<String>,
    /// The caller's metadata object, `{}` when absent.
    pub metadata: Map<String, Value>,
    /// Canonical criterion ids, in the caller's order, repeats kept.
    pub criterion_ids: Vec<String>,
}

/// Every check that needs no store, in the platform's schema order.
pub fn validate(req: Request) -> Result<Valid> {
    let project_id = canonical_id("projectId", &req.project_id)?;
    let work_item_id = req
        .work_item_id
        .as_deref()
        .map(|id| canonical_id("workItemId", id))
        .transpose()?;
    vocabulary("kind", Some(&req.kind), evidence::KINDS)?;
    limits::text("source", &req.source, 1, limits::EVIDENCE_SOURCE_MAX)?;
    if let Some(id) = &req.external_id {
        limits::text("externalId", id, 1, limits::EVIDENCE_EXTERNAL_ID_MAX)?;
    }
    if let Some(url) = &req.url {
        check_url(url)?;
    }
    let repo = req.repo.as_deref().map(canonical_repo).transpose()?;
    let commit_sha = req.commit_sha.as_deref().map(hex_sha).transpose()?;
    let criterion_ids = criterion_ids(&req.criterion_ids)?;
    Ok(Valid {
        raw_project_id: req.project_id,
        project_id,
        raw_work_item_id: req.work_item_id,
        work_item_id,
        kind: req.kind,
        source: req.source,
        external_id: req.external_id,
        url: req.url,
        repo,
        commit_sha,
        metadata: req.metadata.unwrap_or_default(),
        criterion_ids,
    })
}

/// `raw` as a canonical UUID, else `400` on `field`.
pub fn canonical_id(field: &str, raw: &str) -> Result<String> {
    uuid::canonical(raw).ok_or_else(|| ProjectError::invalid_field(field, "invalid").into())
}

/// At most 2048 characters (`422`), and an absolute URL with no surrounding
/// whitespace (`400`), stored as sent. Any scheme parses, as the platform's
/// `z.url()` accepts one: a reader must not render a non-`http(s)` evidence
/// URL as a live link.
fn check_url(url: &str) -> Result<()> {
    if limits::utf16_len(url) > limits::EVIDENCE_URL_MAX {
        let cap = limits::EVIDENCE_URL_MAX;
        return Err(ProjectError::over_limit("url", "too_long", cap).into());
    }
    match url::Url::parse(url) {
        Ok(_) if url.trim() == url => Ok(()),
        _ => Err(ProjectError::invalid_field("url", "invalid").into()),
    }
}

/// A canonical `owner/name`, lowercased, else `422 invalid_format`.
fn canonical_repo(raw: &str) -> Result<String> {
    canonical_github_name(raw.trim()).ok_or_else(|| limits::invariant("repo", "invalid_format"))
}

/// 1–256 hex digits, lowercased; a breach of the length is `422` with the
/// limit, anything else not hex `422 invalid_format`.
fn hex_sha(raw: &str) -> Result<String> {
    limits::text("commitSha", raw, 1, limits::EVIDENCE_EXTERNAL_ID_MAX)?;
    if raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(raw.to_ascii_lowercase())
    } else {
        Err(limits::invariant("commitSha", "invalid_format"))
    }
}

/// At most 20 ids (`422`), each a UUID (`400` on `criterionIds.<index>`),
/// canonical and in the caller's order.
fn criterion_ids(raw: &[String]) -> Result<Vec<String>> {
    if raw.len() > limits::EVIDENCE_CRITERIA_MAX {
        let cap = limits::EVIDENCE_CRITERIA_MAX;
        return Err(ProjectError::over_limit("criterionIds", "too_many", cap).into());
    }
    raw.iter()
        .enumerate()
        .map(|(index, id)| canonical_id(&format!("criterionIds.{index}"), id))
        .collect()
}

#[cfg(test)]
#[path = "tests/evidence_check.rs"]
mod tests;
