//! Drift: what the saved model claims versus what the index holds today.
//!
//! Report-only, like `comemory doctor` — a drifted model is a fact for a human
//! or an agent to act on, not a failing gate. Three questions are asked:
//! which members no longer match a file, which indexed clusters no component
//! covers, and which mined relations the model is missing.

use crate::domains::architecture::cluster::covers;
use crate::domains::architecture::model::{EdgeKind, Model};
use crate::domains::architecture::{current, scaffold};
use crate::prelude::*;
use crate::store::{Connection, indexed_files};

/// A member path that matches no indexed file any more.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StaleMember {
    /// Component declaring it.
    pub component: String,
    /// The member path.
    pub member: String,
}

/// An indexed cluster no component covers.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Unmapped {
    /// Repo-relative cluster path.
    pub path: String,
    /// Summed PageRank of its files.
    pub rank: f64,
    /// Indexed files under it.
    pub files: u32,
}

/// A mined relation between two modeled components that the model omits.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MissingEdge {
    /// Source component id, as declared in the saved model.
    pub from: String,
    /// Destination component id, as declared in the saved model.
    pub to: String,
    /// Every mined kind found between the pair, in `EdgeKind` order. One
    /// omission can be mined under more than one kind — `imports` and
    /// `co_changed` between the same two directories is the common case —
    /// and reporting only the first would hide what the model is missing.
    pub kinds: Vec<EdgeKind>,
    /// The strongest mined weight across those kinds.
    pub weight: i64,
}

/// The whole drift report.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Drift {
    /// Repo the model describes.
    pub repo: String,
    /// Memory id of the model that was checked.
    pub model_id: String,
    /// `stale_members + unmapped + missing_edges`.
    pub drift_count: usize,
    /// Members matching no indexed file.
    pub stale_members: Vec<StaleMember>,
    /// Indexed clusters no component covers.
    pub unmapped: Vec<Unmapped>,
    /// Mined relations the model omits.
    pub missing_edges: Vec<MissingEdge>,
}

/// Compare the saved model for `repo` against a fresh scaffold.
pub fn run(conn: &Connection, repo: &str, opts: &scaffold::Options) -> Result<Drift> {
    let saved = current::require(conn, repo)?;
    let indexed: Vec<String> = indexed_files::list_for_repo(conn, repo)?
        .into_iter()
        .map(|(path, _blob)| path)
        .collect();
    let fresh = scaffold::run(conn, repo, opts)?;

    let stale_members = stale(&saved.model, &indexed);
    let mut unmapped = Vec::new();
    for c in &fresh.components {
        let Some(key) = c.members.first() else {
            continue;
        };
        if owner_of(&saved.model, key).is_none() {
            unmapped.push(Unmapped {
                path: key.clone(),
                rank: c.rank,
                files: c.files,
            });
        }
    }
    let missing_edges = missing(&saved.model, &fresh);

    Ok(Drift {
        repo: repo.to_string(),
        model_id: saved.id,
        drift_count: stale_members.len() + unmapped.len() + missing_edges.len(),
        stale_members,
        unmapped,
        missing_edges,
    })
}

/// Members that no longer cover an indexed file.
fn stale(model: &Model, indexed: &[String]) -> Vec<StaleMember> {
    let mut out = Vec::new();
    for c in &model.components {
        for member in &c.members {
            if !indexed.iter().any(|path| covers(member, path)) {
                out.push(StaleMember {
                    component: c.id.clone(),
                    member: member.clone(),
                });
            }
        }
    }
    out
}

/// Mined relations between two modeled components that the model omits. Two
/// mined kinds between the same pair are one omission, not two, so the pair is
/// reported once with the strongest weight seen.
fn missing(model: &Model, fresh: &Model) -> Vec<MissingEdge> {
    let mut out: Vec<MissingEdge> = Vec::new();
    for e in &fresh.edges {
        let (Some(from), Some(to)) = (
            member_owner(model, fresh, &e.from),
            member_owner(model, fresh, &e.to),
        ) else {
            continue;
        };
        if from == to {
            continue;
        }
        let modeled = model
            .edges
            .iter()
            .any(|saved| saved.from == from && saved.to == to);
        if modeled {
            continue;
        }
        match out.iter_mut().find(|m| m.from == from && m.to == to) {
            Some(seen) => {
                seen.weight = seen.weight.max(e.weight);
                if !seen.kinds.contains(&e.kind) {
                    seen.kinds.push(e.kind);
                    seen.kinds.sort_unstable();
                }
            }
            None => out.push(MissingEdge {
                from: from.to_string(),
                to: to.to_string(),
                kinds: vec![e.kind],
                weight: e.weight,
            }),
        }
    }
    out
}

/// The saved component owning the cluster of the fresh component `id`.
fn member_owner<'a>(model: &'a Model, fresh: &Model, id: &str) -> Option<&'a str> {
    let key = fresh
        .components
        .iter()
        .find(|c| c.id == id)
        .and_then(|c| c.members.first())?;
    owner_of(model, key)
}

/// The saved component whose members overlap `key`, if any.
fn owner_of<'a>(model: &'a Model, key: &str) -> Option<&'a str> {
    model
        .components
        .iter()
        .find(|c| {
            c.members
                .iter()
                .any(|m| covers(m, key) || covers(key, m) || m == key)
        })
        .map(|c| c.id.as_str())
}
