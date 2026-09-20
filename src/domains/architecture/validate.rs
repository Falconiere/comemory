//! Every rule a model must satisfy before it is saved. Validation runs
//! against the **indexed** code — the paths
//! `store::indexed_files::list_for_repo` reports — so a component can never
//! claim files the code index has never seen, and an agent cannot invent a
//! module that does not exist.
//!
//! Each failure is an [`Error::Usage`] (exit 64) naming the offending value:
//! a caller fixing a model needs the offender, not a count.

use std::collections::HashSet;

use crate::domains::architecture::cluster::covers;
use crate::domains::architecture::model::{
    ComponentKind, MAX_BYTES, MAX_COMPONENTS, MAX_EDGES, MAX_SUMMARY, Model, SCHEMA_VERSION,
};
use crate::prelude::*;

/// Validate `model` for `repo` against `indexed` (repo-relative paths of
/// every indexed file). Returns `Ok(())` only when every rule holds.
pub fn validate(model: &Model, repo: &str, indexed: &[String]) -> Result<()> {
    check_envelope(model, repo)?;
    check_size(model)?;
    let group_ids = collect_group_ids(model)?;
    let component_ids = check_components(model, &group_ids, indexed)?;
    check_edges(model, &component_ids)
}

/// Schema version and repo scope — the two fields that decide whether the
/// rest of the document is even about this repository.
fn check_envelope(model: &Model, repo: &str) -> Result<()> {
    if model.schema != SCHEMA_VERSION {
        return Err(Error::Usage(format!(
            "unsupported architecture schema {}, expected {SCHEMA_VERSION}",
            model.schema
        )));
    }
    if model.repo.is_empty() {
        return Err(Error::Usage("architecture model has an empty repo".into()));
    }
    if model.repo != repo {
        return Err(Error::Usage(format!(
            "architecture model is for repo {:?}, not {repo:?}",
            model.repo
        )));
    }
    Ok(())
}

/// The three ceilings: components, edges, and serialized bytes.
fn check_size(model: &Model) -> Result<()> {
    if model.components.len() > MAX_COMPONENTS {
        return Err(Error::Usage(format!(
            "architecture model has {} components, at most {MAX_COMPONENTS} allowed",
            model.components.len()
        )));
    }
    if model.edges.len() > MAX_EDGES {
        return Err(Error::Usage(format!(
            "architecture model has {} edges, at most {MAX_EDGES} allowed",
            model.edges.len()
        )));
    }
    let bytes = serde_json::to_vec(model)?.len();
    if bytes > MAX_BYTES {
        return Err(Error::Usage(format!(
            "architecture model is {bytes} bytes, at most {MAX_BYTES} bytes allowed"
        )));
    }
    Ok(())
}

/// Group ids must be renderer-safe and unique; the set is what a component's
/// `group` is later checked against.
fn collect_group_ids(model: &Model) -> Result<HashSet<&str>> {
    let mut ids = HashSet::new();
    for group in &model.groups {
        check_id(&group.id, "group")?;
        if group.name.is_empty() {
            return Err(Error::Usage(format!(
                "group {} has an empty name",
                group.id
            )));
        }
        if !ids.insert(group.id.as_str()) {
            return Err(Error::Usage(format!("duplicate group id {}", group.id)));
        }
    }
    Ok(ids)
}

/// Component-level rules, including the member-path check against the index.
fn check_components<'a>(
    model: &'a Model,
    group_ids: &HashSet<&str>,
    indexed: &[String],
) -> Result<HashSet<&'a str>> {
    if model.components.is_empty() {
        return Err(Error::Usage("architecture model has no components".into()));
    }
    let mut ids = HashSet::new();
    for c in &model.components {
        check_id(&c.id, "component")?;
        if !ids.insert(c.id.as_str()) {
            return Err(Error::Usage(format!("duplicate component id {}", c.id)));
        }
        if c.name.is_empty() {
            return Err(Error::Usage(format!(
                "component {} has an empty name",
                c.id
            )));
        }
        if c.summary.chars().count() > MAX_SUMMARY {
            return Err(Error::Usage(format!(
                "component {} has a {}-character summary, at most {MAX_SUMMARY} allowed",
                c.id,
                c.summary.chars().count()
            )));
        }
        if let Some(group) = &c.group
            && !group_ids.contains(group.as_str())
        {
            return Err(Error::Usage(format!(
                "component {} names undeclared group {group}",
                c.id
            )));
        }
        check_members(c.id.as_str(), c.kind, &c.members, indexed)?;
    }
    Ok(ids)
}

/// An `external` component owns no repository paths; every other kind owns at
/// least one member, and every member must cover an indexed file.
fn check_members(
    id: &str,
    kind: ComponentKind,
    members: &[String],
    indexed: &[String],
) -> Result<()> {
    if kind == ComponentKind::External {
        if members.is_empty() {
            return Ok(());
        }
        return Err(Error::Usage(format!(
            "external component {id} must own no members, got {}",
            members.join(", ")
        )));
    }
    if members.is_empty() {
        return Err(Error::Usage(format!("component {id} has no members")));
    }
    for member in members {
        if member.is_empty() || member.starts_with('/') || member.contains("..") {
            return Err(Error::Usage(format!(
                "component {id} has an invalid member path {member:?}"
            )));
        }
        if !indexed.iter().any(|path| covers(member, path)) {
            return Err(Error::Usage(format!(
                "component {id} member {member:?} matches no indexed file"
            )));
        }
    }
    Ok(())
}

/// Edge endpoints must be declared components, and no component relates to
/// itself.
fn check_edges(model: &Model, component_ids: &HashSet<&str>) -> Result<()> {
    for e in &model.edges {
        for (role, id) in [("from", &e.from), ("to", &e.to)] {
            if !component_ids.contains(id.as_str()) {
                return Err(Error::Usage(format!(
                    "edge {role} {id:?} is not a declared component"
                )));
            }
        }
        if e.from == e.to {
            return Err(Error::Usage(format!(
                "edge from {} to itself is not a relation",
                e.from
            )));
        }
    }
    Ok(())
}

/// Renderer-safe id: `^[A-Za-z][A-Za-z0-9_]{0,63}$`. Mermaid, DOT and every
/// JS graph library accept this without quoting or escaping.
fn check_id(id: &str, what: &str) -> Result<()> {
    let ok = id.len() <= 64
        && id.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        return Ok(());
    }
    Err(Error::Usage(format!(
        "{what} id {id:?} must match ^[A-Za-z][A-Za-z0-9_]{{0,63}}$"
    )))
}

#[cfg(test)]
#[path = "tests/validate.rs"]
mod tests;
