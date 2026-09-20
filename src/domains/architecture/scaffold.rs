//! The deterministic half of the model: cluster the indexed files of one repo
//! into components, project the mined file-level edges onto them, and emit a
//! [`Model`] an agent can enrich. No LLM, no heuristic beyond the directory
//! tree, and byte-identical output for an unchanged index.

use std::collections::BTreeMap;
use std::path::Path;

use time::OffsetDateTime;

use crate::domains::architecture::cluster;
use crate::domains::architecture::model::{
    Component, ComponentKind, Direction, Edge, EdgeKind, Model, SCHEMA_VERSION, Source,
};
use crate::domains::graph::query::{Rel, build_code_graph};
use crate::prelude::*;
use crate::store::{Connection, indexed_files, memory_row, repo_marker_roots};

/// Scaffold knobs, mirroring the `architecture scaffold` flags.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Directory-prefix depth a component clusters at.
    pub depth: usize,
    /// Truncate to this many components, highest rank first.
    pub max_components: usize,
    /// Drop component edges below this weight.
    pub min_edge_weight: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            depth: 2,
            max_components: 120,
            min_edge_weight: 1,
        }
    }
}

/// One cluster under construction.
#[derive(Default)]
struct Cluster {
    rank: f64,
    files: u32,
}

/// Build the scaffold model for `repo`. Fails with [`Error::Usage`] when the
/// repo has no indexed files, since every later rule is defined against them.
pub fn run(conn: &Connection, repo: &str, opts: &Options) -> Result<Model> {
    let files = indexed_files::list_for_repo(conn, repo)?;
    if files.is_empty() {
        return Err(Error::Usage(format!(
            "no indexed files for repo {repo:?}; run `comemory index-code --repo {repo}` first"
        )));
    }
    let graph = build_code_graph(conn, Some(repo), Rel::All, 1)?;
    let ranks: BTreeMap<&str, f64> = graph
        .nodes
        .iter()
        .map(|n| (n.label.as_str(), n.rank))
        .collect();

    let mut clusters: BTreeMap<String, Cluster> = BTreeMap::new();
    for (path, _blob) in &files {
        let key = cluster::key_for(path, opts.depth);
        let entry = clusters.entry(key).or_default();
        entry.rank += ranks.get(path.as_str()).copied().unwrap_or(0.0);
        entry.files += 1;
    }

    let root = repo_marker_roots::root_path(conn, repo)?;
    let components = components_from(clusters, root.as_deref(), opts.max_components);
    let kept: BTreeMap<&str, &str> = components
        .iter()
        .filter_map(|c| c.members.first().map(|m| (m.as_str(), c.id.as_str())))
        .collect();
    let edges = edges_from(&graph, &kept, opts);

    Ok(Model {
        schema: SCHEMA_VERSION,
        repo: repo.to_string(),
        generated_at: memory_row::iso_format(OffsetDateTime::now_utc())?,
        source: Source::Scaffold,
        direction: Direction::default(),
        groups: Vec::new(),
        components,
        edges,
    })
}

/// Rank-ordered components with unique ids, truncated to `max_components`.
fn components_from(
    clusters: BTreeMap<String, Cluster>,
    root: Option<&str>,
    max_components: usize,
) -> Vec<Component> {
    let mut rows: Vec<(String, Cluster)> = clusters.into_iter().collect();
    rows.sort_by(|a, b| {
        b.1.rank
            .partial_cmp(&a.1.rank)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    rows.truncate(max_components);
    let mut seen: BTreeMap<String, u32> = BTreeMap::new();
    let mut leaves: BTreeMap<&str, u32> = BTreeMap::new();
    for (key, _) in &rows {
        *leaves.entry(leaf_of(key)).or_default() += 1;
    }
    let ambiguous: Vec<String> = leaves
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(leaf, _)| (*leaf).to_string())
        .collect();
    rows.into_iter()
        .map(|(key, c)| {
            let id = unique_id(&key, &mut seen);
            // `src/domains` and `tests/domains` both end in `domains`; a
            // diagram with two nodes called "domains" is unreadable, so an
            // ambiguous leaf falls back to the whole key.
            let leaf = leaf_of(&key);
            let name = if ambiguous.iter().any(|a| a == leaf) {
                key.clone()
            } else {
                leaf.to_string()
            };
            Component {
                name,
                summary: root
                    .and_then(|r| cluster::summary_from_readme(Path::new(r), &key))
                    .unwrap_or_default(),
                id,
                group: None,
                kind: ComponentKind::Module,
                members: vec![key],
                rank: (c.rank * 1e6).round() / 1e6,
                files: c.files,
            }
        })
        .collect()
}

/// The last path segment of a component key.
fn leaf_of(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

/// [`cluster::ident`] with a numeric suffix when two keys sanitize alike.
fn unique_id(key: &str, seen: &mut BTreeMap<String, u32>) -> String {
    let base = cluster::ident(key);
    let count = seen.entry(base.clone()).or_default();
    *count += 1;
    if *count == 1 {
        return base;
    }
    format!("{base}_{count}")
}

/// Project the file-level edges onto the kept components: one component edge
/// per `(from, to, kind)`, weighted by how many file edges contributed.
fn edges_from(
    graph: &crate::domains::graph::code_graph::CodeGraph,
    kept: &BTreeMap<&str, &str>,
    opts: &Options,
) -> Vec<Edge> {
    let path_of: BTreeMap<&str, &str> = graph
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    let mut counts: BTreeMap<(String, String, EdgeKind), i64> = BTreeMap::new();
    for e in &graph.edges {
        let (Some(src), Some(dst)) = (path_of.get(e.src.as_str()), path_of.get(e.dst.as_str()))
        else {
            continue;
        };
        let (Some(from), Some(to)) = (
            component_of(src, kept, opts.depth),
            component_of(dst, kept, opts.depth),
        ) else {
            continue;
        };
        if from == to {
            continue;
        }
        let Some(kind) = edge_kind(&e.rel) else {
            continue;
        };
        *counts
            .entry((from.to_string(), to.to_string(), kind))
            .or_default() += 1;
    }
    let mut edges: Vec<Edge> = counts
        .into_iter()
        .filter(|(_, weight)| *weight >= opts.min_edge_weight)
        .map(|((from, to, kind), weight)| Edge {
            from,
            to,
            kind,
            weight,
        })
        .collect();
    edges.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then_with(|| a.from.cmp(&b.from))
            .then_with(|| a.to.cmp(&b.to))
    });
    edges
}

/// The kept component id owning `path`, if any.
fn component_of<'a>(path: &str, kept: &BTreeMap<&str, &'a str>, depth: usize) -> Option<&'a str> {
    kept.get(cluster::key_for(path, depth).as_str()).copied()
}

/// The model's edge kind for a mined `edges.edge_kind` string.
fn edge_kind(rel: &str) -> Option<EdgeKind> {
    match rel {
        "imports" => Some(EdgeKind::Imports),
        "co_changed" => Some(EdgeKind::CoChanged),
        _ => None,
    }
}
