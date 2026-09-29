//! Typed evidence (#346), shared by the attach and page cores: the closed
//! kind and trust vocabularies, the claim-shape check and the initial trust
//! it earns, the stored `metadata` shape, and the wire view. Ported from the
//! platform's `evidence-kinds.ts`, `evidence-trust.ts`,
//! `project-evidence-claim.ts` and `project-evidence-view.ts` (comemory.io
//! `b86dec5`), minus the verifier: no claim is resolved here, so a claim a
//! verifier could check stays `pending` until #348 runs.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::domains::projects::limits;
use crate::domains::projects::timestamp::iso;
use crate::prelude::*;
use crate::store::project_evidence::{EvidenceRow, TrustFilter};
use crate::utilities::project_error::ProjectError;

/// The evidence kinds, ported unchanged.
pub const KINDS: &[&str] = &[
    "commit",
    "pull_request",
    "test_run",
    "deployment",
    "session",
    "decision",
    "memory",
    "external_url",
];

/// The trust states, most trusted first.
pub const TRUSTS: &[&str] = &["verified", "self_reported", "pending", "invalid"];

/// The least-trusted state: what a stored value outside [`TRUSTS`] reads as.
const LEAST_TRUSTED: &str = "invalid";

/// The `metadata.reason` a claim waiting for its verifier records.
pub const PENDING_REASON: &str = "verification_pending";

/// `stored` when it is a trust state this build knows, else the least
/// trusted one, which satisfies nothing.
#[must_use]
pub fn read_trust(stored: &str) -> &str {
    if TRUSTS.contains(&stored) {
        stored
    } else {
        LEAST_TRUSTED
    }
}

/// The stored values a `trust` filter keeps: exactly the named state, except
/// that the least-trusted one also keeps every value [`read_trust`] maps to it.
#[must_use]
pub fn trust_filter(trust: &str) -> TrustFilter<'_> {
    if trust == LEAST_TRUSTED {
        TrustFilter::NoneOf(&TRUSTS[..TRUSTS.len() - 1])
    } else {
        TrustFilter::Exactly(trust)
    }
}

/// The provider-relevant half of an attach: what decides whether a claim
/// could ever be verified. `repo` is canonical and `commit_sha` lowercase.
#[derive(Debug, Clone, Copy)]
pub struct Claim<'a> {
    /// A member of [`KINDS`].
    pub kind: &'a str,
    /// Canonical `owner/name`, when named.
    pub repo: Option<&'a str>,
    /// Lowercase hex, when named.
    pub commit_sha: Option<&'a str>,
    /// The id in the source system, when named.
    pub external_id: Option<&'a str>,
}

/// The initial trust of a well-shaped claim, and its `metadata.reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Initial {
    /// `pending` or `self_reported`.
    pub trust: &'static str,
    /// Why it is `pending`; `None` otherwise.
    pub reason: Option<&'static str>,
}

impl Claim<'_> {
    /// Refuse a claim whose own shape makes it unverifiable, as the
    /// platform's `assertEvidenceClaimShape` does (`422 invalid_request`
    /// naming the field and the reason); a claim no verifier covers needs
    /// nothing. The platform's exact 40-hex SHA is the issue's hex 1–256.
    pub fn check_shape(&self) -> Result<()> {
        let sha = || required(self.commit_sha, "commitSha", "required_hex_sha");
        match self.kind {
            "commit" => {
                required(self.repo, "repo", "required_for_commit")?;
                sha()
            }
            "pull_request" => {
                required(self.repo, "repo", "required_for_pull_request")?;
                let number = self
                    .external_id
                    .filter(|id| pull_request_number(id).is_some());
                required(number, "externalId", "required_pull_request_number")
            }
            "test_run" if self.repo.is_some() => {
                sha()?;
                required(self.external_id, "externalId", "required_check_run_name")
            }
            "session" | "decision" | "memory" => {
                required(self.external_id, "externalId", "required_record_id")
            }
            _ => Ok(()),
        }
    }

    /// `pending` when a verifier could check this claim (the platform's
    /// providers: a repository object, or a record the workspace holds), else
    /// `self_reported`. Assumes [`Claim::check_shape`] passed.
    #[must_use]
    pub fn initial(&self) -> Initial {
        let verifiable = match self.kind {
            "commit" | "pull_request" | "session" | "decision" | "memory" => true,
            "test_run" => self.repo.is_some(),
            _ => false,
        };
        if verifiable {
            Initial {
                trust: "pending",
                reason: Some(PENDING_REASON),
            }
        } else {
            Initial {
                trust: "self_reported",
                reason: None,
            }
        }
    }
}

/// `422` on `field` for `reason` when the claim leaves `value` out.
fn required(value: Option<&str>, field: &str, reason: &str) -> Result<()> {
    match value {
        Some(_) => Ok(()),
        None => Err(limits::invariant(field, reason)),
    }
}

/// A positive pull-request number, when `external_id` is one.
fn pull_request_number(external_id: &str) -> Option<u64> {
    if !external_id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    external_id.parse().ok().filter(|n| *n > 0)
}

/// What `project_evidence.metadata` holds, the platform's
/// `StoredEvidenceMetadataSchema`: the claim's own repository and commit, why
/// it is `pending`, the caller's metadata, and what a provider reported.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StoredMetadata<'a> {
    /// Canonical `owner/name`, when the claim named one.
    pub repo: Option<&'a str>,
    /// Lowercase hex, when the claim named one.
    pub commit_sha: Option<&'a str>,
    /// Why the row is `pending`; `null` otherwise.
    pub reason: Option<&'a str>,
    /// The caller's metadata object.
    pub claim: &'a Map<String, Value>,
    /// What a provider reported; `null` until #348 verifies.
    pub provider: Option<&'a Value>,
}

impl StoredMetadata<'_> {
    /// The encoded column, refused over the 16 KiB cap as encoded bytes.
    pub fn encode(&self) -> Result<String> {
        let encoded = serde_json::to_string(self)?;
        if encoded.len() > limits::EVIDENCE_METADATA_BYTES_MAX {
            let cap = limits::EVIDENCE_METADATA_BYTES_MAX;
            return Err(ProjectError::over_limit("metadata", "too_large", cap).into());
        }
        Ok(encoded)
    }
}

/// One evidence record on the wire, the platform's
/// `ProjectEvidenceViewSchema` key for key.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceView {
    /// Evidence UUID.
    pub id: String,
    /// The work item it supports; `null` for the project.
    pub work_item_id: Option<String>,
    /// The execution that produced it; `null` outside one.
    pub execution_id: Option<String>,
    /// Its kind.
    pub kind: String,
    /// The system it came from.
    pub source: String,
    /// Id in that system.
    pub external_id: Option<String>,
    /// Link to it.
    pub url: Option<String>,
    /// Trust, an unrecognised stored value read as `invalid`.
    pub trust: String,
    /// The stored metadata object.
    pub metadata: Map<String, Value>,
    /// Content hash, when known.
    pub content_hash: Option<String>,
    /// Verifier identity.
    pub verified_by: Option<String>,
    /// ISO-8601, when verified.
    pub verified_at: Option<String>,
    /// `user` or `project_agent`.
    pub creator_principal_type: String,
    /// The creator's principal id.
    pub creator_principal_id: String,
    /// ISO-8601.
    pub created_at: String,
}

/// One stored row as its view. Metadata that is not a JSON object reads as
/// `{}` with a warning naming the row, never the value; a timestamp outside
/// the representable range is a corrupt row, refused.
pub fn view(row: EvidenceRow) -> Result<EvidenceView> {
    let metadata = if let Ok(Value::Object(map)) = serde_json::from_str(&row.metadata) {
        map
    } else {
        tracing::warn!(
            row_id = %row.id,
            column = "project_evidence.metadata",
            "stored metadata is not a JSON object; reading as {{}}"
        );
        Map::new()
    };
    let verified_at = row
        .verified_at
        .map(|ms| timestamp(&row.id, "verified_at", ms))
        .transpose()?;
    Ok(EvidenceView {
        created_at: timestamp(&row.id, "created_at", row.created_at)?,
        trust: read_trust(&row.trust).to_string(),
        id: row.id,
        work_item_id: row.work_item_id,
        execution_id: row.execution_id,
        kind: row.kind,
        source: row.source,
        external_id: row.external_id,
        url: row.url,
        metadata,
        content_hash: row.content_hash,
        verified_by: row.verified_by,
        verified_at,
        creator_principal_type: row.creator_principal_type,
        creator_principal_id: row.creator_principal_id,
    })
}

/// `ms` as ISO-8601, or the corrupt-row invariant naming `id` and `column`.
fn timestamp(id: &str, column: &str, ms: i64) -> Result<String> {
    iso(ms).ok_or_else(|| {
        ProjectError::Invariant {
            invariant: "project_timestamp_range".to_string(),
            message: format!(
                "project_evidence.{column} of {id} is outside the representable range"
            ),
        }
        .into()
    })
}

#[cfg(test)]
#[path = "tests/evidence.rs"]
mod tests;
