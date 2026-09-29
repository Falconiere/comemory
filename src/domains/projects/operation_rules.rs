//! The request-level rules a parsed operation list must meet before it is
//! stored, and its normalization: every UUID lowercase canonical, every
//! `targetDate` rendered as `toISOString()` (the platform's
//! `z.coerce.date()` followed by `JSON.stringify`). A bad UUID or date is a
//! schema-edge `400`; a length, range or proposal cap a `422`, each naming
//! the field by its platform path (`operations.3.workItem.title`).
//!
//! References, entity caps after applying and the dependency graph are the
//! live-plan preflight's (#337), not these rules'.

use serde_json::Value;

use crate::domains::projects::limits::{self, text};
use crate::domains::projects::operation_fields::{
    MilestonePatch, ProjectPatch, ProposedCriterion, ProposedMilestone,
};
use crate::domains::projects::operations::{Nullable, Operation};
use crate::domains::projects::timestamp;
use crate::domains::projects::work_item_rules;
use crate::prelude::*;
use crate::utilities::ordered_details::OrderedDetails;
use crate::utilities::project_error::{ProjectError, RequestEdge};
use crate::utilities::uuid;

/// The documented cap on operations per proposal.
pub const OPERATIONS_MAX: usize = 200;
/// The documented cap on a proposal's serialized operations, in bytes.
pub const OPERATIONS_BYTES_MAX: usize = 256 * 1024;

/// Check and normalize `operations` in place: at least one, at most
/// [`OPERATIONS_MAX`], every operation's fields, then the serialized size of
/// the normalized list against [`OPERATIONS_BYTES_MAX`].
pub fn validate(operations: &mut [Operation]) -> Result<()> {
    if operations.is_empty() {
        return Err(ProjectError::over_limit("operations", "too_short", 1).into());
    }
    if operations.len() > OPERATIONS_MAX {
        return Err(cap("operations", OPERATIONS_MAX, operations.len()));
    }
    for (index, operation) in operations.iter_mut().enumerate() {
        check(&format!("operations.{index}"), operation)?;
    }
    let bytes = serde_json::to_vec(&*operations)?.len();
    if bytes > OPERATIONS_BYTES_MAX {
        return Err(cap("operations.bytes", OPERATIONS_BYTES_MAX, bytes));
    }
    Ok(())
}

/// The platform's cap refusal (`refuseCap`): `422`, details `{field,
/// reason: cap_exceeded, limit, actual}`; the message also names the limit.
fn cap(field: &'static str, limit: usize, actual: usize) -> Error {
    ProjectError::InvalidRequest {
        edge: RequestEdge::Invariant,
        message: format!("This proposal exceeds the {field} limit of {limit}"),
        details: OrderedDetails::from_pairs(vec![
            ("field", Value::from(field)),
            ("reason", Value::from("cap_exceeded")),
            ("limit", Value::from(limit)),
            ("actual", Value::from(actual)),
        ]),
    }
    .into()
}

/// One operation at `at`.
fn check(at: &str, operation: &mut Operation) -> Result<()> {
    let at = |suffix: &str| format!("{at}.{suffix}");
    match operation {
        Operation::ProjectUpdate { patch } => project(&at("patch"), patch),
        Operation::CriterionCreate { criterion: c } => proposed_criterion(&at("criterion"), c),
        Operation::CriterionUpdate {
            criterion_id,
            patch,
        } => {
            id(&at("criterionId"), criterion_id)?;
            criterion(&at("patch"), patch.description.as_deref(), patch.position)
        }
        Operation::MilestoneCreate { milestone: m } => proposed_milestone(&at("milestone"), m),
        Operation::MilestoneUpdate {
            milestone_id,
            patch,
        } => {
            id(&at("milestoneId"), milestone_id)?;
            milestone_patch(&at("patch"), patch)
        }
        Operation::WorkItemCreate { work_item: w } => work_item_rules::create(&at("workItem"), w),
        Operation::WorkItemUpdate {
            work_item_id,
            patch,
        } => {
            id(&at("workItemId"), work_item_id)?;
            work_item_rules::patch(&at("patch"), patch)
        }
        Operation::CriterionArchive { criterion_id: c } => id(&at("criterionId"), c),
        Operation::MilestoneArchive { milestone_id: m } => id(&at("milestoneId"), m),
        Operation::WorkItemArchive { work_item_id: w } => id(&at("workItemId"), w),
        Operation::DependencyAdd {
            blocker_id,
            blocked_id,
        }
        | Operation::DependencyRemove {
            blocker_id,
            blocked_id,
        } => {
            id(&at("blockerId"), blocker_id)?;
            id(&at("blockedId"), blocked_id)
        }
    }
}

/// `project.update`'s charter patch.
fn project(at: &str, patch: &mut ProjectPatch) -> Result<()> {
    let name = patch.name.as_deref();
    optional_text(&format!("{at}.name"), name, 1, limits::NAME_MAX)?;
    let outcome = patch.outcome.as_deref();
    optional_text(&format!("{at}.outcome"), outcome, 1, limits::OUTCOME_MAX)?;
    let lists = [
        (
            "constraints",
            &patch.constraints,
            limits::CONSTRAINTS_MAX_COUNT,
        ),
        ("nonGoals", &patch.non_goals, limits::NON_GOALS_MAX_COUNT),
    ];
    for (field, items, max) in lists {
        if let Some(items) = items {
            limits::list(&format!("{at}.{field}"), items, max, limits::CONSTRAINT_MAX)?;
        }
    }
    if let Some(repositories) = &patch.repositories {
        let field = format!("{at}.repositories");
        limits::list(&field, repositories, limits::REPOSITORIES_MAX, usize::MAX)?;
    }
    if let Some(target) = patch.target_date.as_mut().and_then(Nullable::as_mut) {
        date(&format!("{at}.targetDate"), target)?;
    }
    Ok(())
}

/// `criterion.create`'s new criterion.
fn proposed_criterion(at: &str, c: &mut ProposedCriterion) -> Result<()> {
    id(&format!("{at}.id"), &mut c.id)?;
    if let Some(item) = c.work_item_id.as_mut().and_then(Nullable::as_mut) {
        id(&format!("{at}.workItemId"), item)?;
    }
    criterion(at, Some(&c.description), c.position)
}

/// A criterion's description and position.
fn criterion(at: &str, description: Option<&str>, position: Option<i64>) -> Result<()> {
    let max = limits::CRITERION_DESCRIPTION_MAX;
    optional_text(&format!("{at}.description"), description, 1, max)?;
    non_negative(&format!("{at}.position"), position)
}

/// `milestone.create`'s new milestone.
fn proposed_milestone(at: &str, m: &mut ProposedMilestone) -> Result<()> {
    id(&format!("{at}.id"), &mut m.id)?;
    let fields = (Some(m.name.as_str()), m.description.as_deref());
    milestone(at, fields, Some(&mut m.target_date), m.position)
}

/// `milestone.update`'s patch.
fn milestone_patch(at: &str, m: &mut MilestonePatch) -> Result<()> {
    let fields = (m.name.as_deref(), m.description.as_deref());
    milestone(at, fields, m.target_date.as_mut(), m.position)
}

/// A milestone's `(name, description)`, target date and position.
fn milestone(
    at: &str,
    (name, description): (Option<&str>, Option<&str>),
    target_date: Option<&mut String>,
    position: Option<i64>,
) -> Result<()> {
    optional_text(&format!("{at}.name"), name, 1, limits::NAME_MAX)?;
    let max = limits::OUTCOME_MAX;
    optional_text(&format!("{at}.description"), description, 0, max)?;
    if let Some(target) = target_date {
        date(&format!("{at}.targetDate"), target)?;
    }
    non_negative(&format!("{at}.position"), position)
}

/// [`text`] when `value` is present.
pub fn optional_text(field: &str, value: Option<&str>, min: usize, max: usize) -> Result<()> {
    value.map_or(Ok(()), |v| text(field, v, min, max))
}

/// A number that must be at least zero: `422` `too_small` otherwise.
pub fn non_negative(field: &str, value: Option<i64>) -> Result<()> {
    match value {
        Some(n) if n < 0 => Err(ProjectError::over_limit(field, "too_small", 0).into()),
        _ => Ok(()),
    }
}

/// A UUID, rewritten lowercase canonical; anything else is `400`.
pub fn id(field: &str, value: &mut String) -> Result<()> {
    *value = uuid::canonical(value)
        .ok_or_else(|| Error::from(ProjectError::invalid_field(field, "invalid")))?;
    Ok(())
}

/// A `targetDate`, rewritten as `toISOString()`; anything else is `400`.
fn date(field: &str, value: &mut String) -> Result<()> {
    let invalid = || Error::from(ProjectError::invalid_field(field, "invalid"));
    let ms = timestamp::parse_target_date(value).map_err(|_| invalid())?;
    *value = timestamp::iso(ms).ok_or_else(invalid)?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/operation_rules.rs"]
mod tests;
