//! A work item's request-level rules, shared by `work_item.create` and
//! `work_item.update`: one field list, checked once, so a create and a
//! patch can never be held to different caps.

use crate::domains::projects::limits;
use crate::domains::projects::operation_rules::{id, non_negative, optional_text};
use crate::domains::projects::operations::Nullable;
use crate::domains::projects::work_item_fields::{Assignee, ProposedWorkItem, WorkItemPatch};
use crate::prelude::*;

/// The platform's `WORK_ITEM_DESCRIPTION_MAX`.
pub const DESCRIPTION_MAX: usize = 8000;

/// The checked fields of either shape; the UUIDs are borrowed mutably so
/// they are normalized in place.
struct Fields<'a> {
    parent: Option<&'a mut String>,
    milestone: Option<&'a mut String>,
    title: Option<&'a str>,
    description: Option<&'a str>,
    estimate: Option<i64>,
    assignee: Option<&'a Assignee>,
    repo: Option<&'a str>,
    position: Option<i64>,
}

/// `work_item.create`'s new item at `at`.
pub fn create(at: &str, w: &mut ProposedWorkItem) -> Result<()> {
    id(&format!("{at}.id"), &mut w.id)?;
    check(
        at,
        Fields {
            parent: present(&mut w.parent_work_item_id),
            milestone: present(&mut w.milestone_id),
            title: Some(&w.title),
            description: w.description.as_deref(),
            estimate: set(w.estimate.as_ref()).copied(),
            assignee: set(w.assignee.as_ref()),
            repo: set(w.repo.as_ref()).map(String::as_str),
            position: w.position,
        },
    )
}

/// `work_item.update`'s patch at `at`.
pub fn patch(at: &str, w: &mut WorkItemPatch) -> Result<()> {
    let title = w.title.as_deref();
    let (assignee, repo) = (
        set(w.assignee.as_ref()),
        set(w.repo.as_ref()).map(String::as_str),
    );
    let (description, estimate, position) = (
        w.description.as_deref(),
        set(w.estimate.as_ref()),
        w.position,
    );
    check(
        at,
        Fields {
            parent: present(&mut w.parent_work_item_id),
            milestone: present(&mut w.milestone_id),
            title,
            description,
            estimate: estimate.copied(),
            assignee,
            repo,
            position,
        },
    )
}

/// A set (non-`null`) nullable value.
fn present(value: &mut Option<Nullable<String>>) -> Option<&mut String> {
    value.as_mut().and_then(Nullable::as_mut)
}

/// A set (non-`null`) nullable value, shared.
fn set<T>(value: Option<&Nullable<T>>) -> Option<&T> {
    value.and_then(Nullable::as_ref)
}

/// Every rule, in the platform's field order.
fn check(at: &str, f: Fields<'_>) -> Result<()> {
    let at = |field: &str| format!("{at}.{field}");
    if let Some(parent) = f.parent {
        id(&at("parentWorkItemId"), parent)?;
    }
    if let Some(milestone) = f.milestone {
        id(&at("milestoneId"), milestone)?;
    }
    optional_text(&at("title"), f.title, 1, limits::NAME_MAX)?;
    optional_text(&at("description"), f.description, 0, DESCRIPTION_MAX)?;
    non_negative(&at("estimate"), f.estimate)?;
    if let Some(assignee) = f.assignee {
        let field = at("assignee.principalId");
        limits::text(&field, &assignee.principal_id, 1, usize::MAX)?;
    }
    optional_text(&at("repo"), f.repo, 1, usize::MAX)?;
    non_negative(&at("position"), f.position)
}
