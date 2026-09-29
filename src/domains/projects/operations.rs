//! The twelve typed plan operations a proposal is written in, ported
//! verbatim from the platform's `project-plan-operations.ts`, discriminated
//! on `op`. Every create names a client-generated UUID, so approval writes
//! the exact identity the reviewer saw. Shapes and enums are checked here;
//! lengths, ranges, normalization and the proposal caps are
//! [`super::operation_rules`].

use std::fmt;

use serde::de::{self, Deserializer, IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};

use crate::domains::projects::operation_fields::{
    CriterionPatch, MilestonePatch, ProjectPatch, ProposedCriterion, ProposedMilestone,
};
use crate::domains::projects::work_item_fields::{ProposedWorkItem, WorkItemPatch};

/// The schema edge's outer bound on `operations`: ten times the documented
/// 200-operation cap. Past it the list is refused `400` at the schema edge.
pub const OPERATIONS_SCHEMA_MAX: usize = 2000;

/// One typed plan operation.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "op", deny_unknown_fields)]
pub enum Operation {
    /// Change charter fields.
    #[serde(rename = "project.update")]
    ProjectUpdate {
        /// The fields to change.
        patch: ProjectPatch,
    },
    /// Add a criterion.
    #[serde(rename = "criterion.create")]
    CriterionCreate {
        /// The new criterion.
        criterion: ProposedCriterion,
    },
    /// Change a criterion.
    #[serde(rename = "criterion.update", rename_all = "camelCase")]
    CriterionUpdate {
        /// Its UUID.
        criterion_id: String,
        /// The fields to change.
        patch: CriterionPatch,
    },
    /// Archive a criterion.
    #[serde(rename = "criterion.archive", rename_all = "camelCase")]
    CriterionArchive {
        /// Its UUID.
        criterion_id: String,
    },
    /// Add a milestone.
    #[serde(rename = "milestone.create")]
    MilestoneCreate {
        /// The new milestone.
        milestone: ProposedMilestone,
    },
    /// Change a milestone.
    #[serde(rename = "milestone.update", rename_all = "camelCase")]
    MilestoneUpdate {
        /// Its UUID.
        milestone_id: String,
        /// The fields to change.
        patch: MilestonePatch,
    },
    /// Archive a milestone.
    #[serde(rename = "milestone.archive", rename_all = "camelCase")]
    MilestoneArchive {
        /// Its UUID.
        milestone_id: String,
    },
    /// Add a work item.
    #[serde(rename = "work_item.create", rename_all = "camelCase")]
    WorkItemCreate {
        /// The new work item.
        work_item: ProposedWorkItem,
    },
    /// Change a work item.
    #[serde(rename = "work_item.update", rename_all = "camelCase")]
    WorkItemUpdate {
        /// Its UUID.
        work_item_id: String,
        /// The fields to change.
        patch: WorkItemPatch,
    },
    /// Archive a work item.
    #[serde(rename = "work_item.archive", rename_all = "camelCase")]
    WorkItemArchive {
        /// Its UUID.
        work_item_id: String,
    },
    /// Add a `blocks` edge.
    #[serde(rename = "dependency.add", rename_all = "camelCase")]
    DependencyAdd {
        /// The item that blocks.
        blocker_id: String,
        /// The item that is blocked.
        blocked_id: String,
    },
    /// Remove a `blocks` edge.
    #[serde(rename = "dependency.remove", rename_all = "camelCase")]
    DependencyRemove {
        /// The item that blocks.
        blocker_id: String,
        /// The item that is blocked.
        blocked_id: String,
    },
}

/// A `.nullable()` field that is present: an explicit `null`, or a value.
/// As `Option<Nullable<T>>`, an absent key (`None`) leaves the stored value
/// and `Some(Null)` clears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nullable<T> {
    /// `null`.
    Null,
    /// A value.
    Value(T),
}

impl<T> Nullable<T> {
    /// The value, unless `null`.
    pub fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Null => None,
            Self::Value(value) => Some(value),
        }
    }

    /// The value, mutably, unless `null`.
    pub fn as_mut(&mut self) -> Option<&mut T> {
        match self {
            Self::Null => None,
            Self::Value(value) => Some(value),
        }
    }
}

impl<T: Serialize> Serialize for Nullable<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_none(),
            Self::Value(value) => serializer.serialize_some(value),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Nullable<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(deserializer).map(|value| value.map_or(Self::Null, Self::Value))
    }
}

impl<T: schemars::JsonSchema> schemars::JsonSchema for Nullable<T> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        Option::<T>::schema_name()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        Option::<T>::json_schema(generator)
    }

    fn inline_schema() -> bool {
        Option::<T>::inline_schema()
    }
}

/// `deserialize_with` for every optional operation field: a present key is
/// `Some`, and only an absent one stays `None` (via `default`) — serde's own
/// `Option` would read `null` as absent and silently drop the key. So `null`
/// on a plain `T` is refused, as zod's `.optional()` refuses it, while an
/// `Option<Nullable<T>>` field reads it as [`Nullable::Null`].
pub fn non_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// `deserialize_with` for `operations`: at most [`OPERATIONS_SCHEMA_MAX`]
/// elements. A longer list is refused from its length hint before any
/// element is read, or on the first element past the bound, with the
/// error at the list's own path (`operations is invalid`).
pub fn bounded<'de, D>(deserializer: D) -> Result<Vec<Operation>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_seq(Bounded)
}

/// The visitor behind [`bounded`].
struct Bounded;

impl<'de> Visitor<'de> for Bounded {
    type Value = Vec<Operation>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at most {OPERATIONS_SCHEMA_MAX} plan operations")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let too_many = || de::Error::custom("too many plan operations");
        if seq.size_hint().is_some_and(|n| n > OPERATIONS_SCHEMA_MAX) {
            return Err(too_many());
        }
        let mut operations = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while operations.len() < OPERATIONS_SCHEMA_MAX {
            match seq.next_element()? {
                Some(operation) => operations.push(operation),
                None => return Ok(operations),
            }
        }
        match seq.next_element::<IgnoredAny>()? {
            Some(_) => Err(too_many()),
            None => Ok(operations),
        }
    }
}

#[cfg(test)]
#[path = "tests/operations.rs"]
mod tests;
