//! A create request checked against every charter rule before any store
//! access, in the platform's field order, so the one refusal a caller sees
//! is deterministic: `id`, `name`, `keyPrefix`, `outcome`, `successCriteria`,
//! `constraints`, `nonGoals`, `repositories`, `leadUserId`, `targetDate`.

use crate::domains::projects::create::Request;
use crate::domains::projects::limits::{self, key_prefix, list, text};
use crate::domains::projects::principal::{Principal, PrincipalType};
use crate::domains::projects::timestamp::parse_target_date;
use crate::domains::sync::repository_identity::canonical_github_name;
use crate::prelude::*;
use crate::utilities::project_error::ProjectError;
use crate::utilities::uuid;

/// A charter that passed every check, normalized for storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Charter {
    /// Lowercase UUID: the caller's, or freshly minted.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Work-item key prefix.
    pub key_prefix: String,
    /// The finite outcome.
    pub outcome: String,
    /// Project-level success criteria, in order.
    pub success_criteria: Vec<String>,
    /// Constraints.
    pub constraints: Vec<String>,
    /// Non-goals.
    pub non_goals: Vec<String>,
    /// Canonical, de-duplicated `owner/name` repositories, in first-seen order.
    pub repositories: Vec<String>,
    /// The lead, always a human principal.
    pub lead: Principal,
    /// Epoch milliseconds, when set.
    pub target_date: Option<i64>,
}

/// Check `req` for `actor`; the lead defaults to the actor's id.
pub fn validate(req: Request, actor: &Principal) -> Result<Charter> {
    let id = match req.id.as_deref() {
        Some(raw) => {
            uuid::canonical(raw).ok_or_else(|| limits::invariant("id", "invalid_format"))?
        }
        None => uuid::new_v4()?,
    };
    text("name", &req.name, 1, limits::NAME_MAX)?;
    key_prefix(&req.key_prefix)?;
    text("outcome", &req.outcome, 1, limits::OUTCOME_MAX)?;
    list(
        "successCriteria",
        &req.success_criteria,
        limits::CRITERIA_MAX,
        limits::CRITERION_DESCRIPTION_MAX,
    )?;
    list(
        "constraints",
        &req.constraints,
        limits::CONSTRAINTS_MAX_COUNT,
        limits::CONSTRAINT_MAX,
    )?;
    list(
        "nonGoals",
        &req.non_goals,
        limits::NON_GOALS_MAX_COUNT,
        limits::CONSTRAINT_MAX,
    )?;
    let repositories = repositories(&req.repositories)?;
    let lead = match req.lead_user_id.as_deref() {
        Some(lead) => {
            text("leadUserId", lead, 1, usize::MAX)?;
            Principal::new(PrincipalType::User, lead)
        }
        None => Principal::new(PrincipalType::User, &actor.id),
    };
    let target_date = req
        .target_date
        .as_deref()
        .map(parse_target_date)
        .transpose()?;
    Ok(Charter {
        id,
        name: req.name,
        key_prefix: req.key_prefix,
        outcome: req.outcome,
        success_criteria: req.success_criteria,
        constraints: req.constraints,
        non_goals: req.non_goals,
        repositories,
        lead,
        target_date,
    })
}

/// At most 50 entries (counted before de-duplication, as the platform
/// does), each a canonical `owner/name` after trimming; returned lowercased
/// and de-duplicated in first-seen order. The allow-list is #337's.
fn repositories(raw: &[String]) -> Result<Vec<String>> {
    if raw.len() > limits::REPOSITORIES_MAX {
        let refusal =
            ProjectError::over_limit("repositories", "too_many", limits::REPOSITORIES_MAX);
        return Err(refusal.into());
    }
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    for (index, entry) in raw.iter().enumerate() {
        let field = format!("repositories.{index}");
        let canonical = canonical_github_name(entry.trim())
            .ok_or_else(|| limits::invariant(&field, "invalid_format"))?;
        if !out.contains(&canonical) {
            out.push(canonical);
        }
    }
    Ok(out)
}
