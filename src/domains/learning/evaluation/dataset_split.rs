//! Grouped split assignment for the reviewed dataset export (#210).
//!
//! A row links one query to one piece of content, so either side can leak. The
//! assignment is therefore made to a CONNECTED COMPONENT of the bipartite
//! query/content graph rather than to a row or to a query: a document two
//! queries both retrieved, and a query two content versions both answered, stay
//! on one side of the split by construction.
//!
//! Assignment is a seeded hash of the component's id, not a shuffle, so an
//! existing component keeps its split as the corpus grows. The realized ratios
//! therefore only approach the requested ones, and the manifest reports the
//! realized counts as the truth.

use std::collections::BTreeMap;

use crate::domains::learning::evaluation::candidate_identity::CandidateIdentity;
use crate::domains::learning::evaluation::dataset_record::Split;
use crate::utilities::digest::sha256_hex;

/// How many hex characters of a digest become an id or a hash bucket.
const ID_HEX: usize = 16;

/// The split configuration one export ran under.
#[derive(Debug, Clone)]
pub struct SplitConfig {
    /// Salt for the group-id hash. Changing it reassigns every component.
    pub seed: String,
    /// Train, validation and holdout ratios, in that order.
    pub ratios: [f64; 3],
    /// Force every component holding a code candidate from this repo into the
    /// holdout split — the "reserve a separate repository" rule.
    pub holdout_repo: Option<String>,
    /// Force every component holding an observation captured at or after this
    /// instant into the holdout split — the "reserve a time slice" rule.
    pub holdout_since: Option<String>,
}

/// What one row contributes to the graph.
pub struct GroupInput<'a> {
    /// The row's query key.
    pub query_group: &'a str,
    /// The row's content key.
    pub content_group: &'a str,
    /// The repo label of a code candidate; `None` for every other domain.
    pub repo: Option<&'a str>,
    /// The `at` of the observation this row came from.
    pub at: &'a str,
}

/// One component's assignment, as the manifest reports it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupAssignment {
    /// `g-<16hex>` over the component's sorted member keys.
    pub group_id: String,
    /// The split the whole component landed in.
    pub split: Split,
    /// `"repo"` or `"since"` when a reservation rule overrode the hash.
    pub forced: Option<String>,
    /// Distinct query keys in the component.
    pub queries: usize,
    /// Distinct content keys in the component.
    pub contents: usize,
    /// Rows in the component.
    pub rows: usize,
}

/// Every row's component and split, plus the per-component report.
pub struct SplitPlan {
    /// One `(group_id, split)` per input row, in the input's own order.
    pub per_row: Vec<(String, Split)>,
    /// Every component, ascending by `group_id`.
    pub assignments: Vec<GroupAssignment>,
}

/// The near-duplicate key of one query: its lowercased alphanumeric tokens,
/// sorted and deduplicated, digested.
///
/// "frontmatter contract", "Frontmatter, contract?" and "contract frontmatter"
/// therefore share a key. Coarse on purpose: the alternative, a SimHash radius,
/// makes membership depend on which other queries happen to be present, which
/// would move an existing component between splits as the corpus grows.
pub fn query_group(query: &str) -> String {
    let mut tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect();
    tokens.sort_unstable();
    tokens.dedup();
    let digest = sha256_hex(tokens.join(" ").as_bytes());
    format!("qg-{}", digest.get(..ID_HEX).unwrap_or(digest.as_str()))
}

/// The content key of one candidate: its identity with the content version and
/// the intra-file position removed.
///
/// Two content versions of one memory, two symbols of one source file and two
/// chunks of one document therefore share a key, which is what stops a
/// near-duplicate passage from appearing on both sides of a split. The key is
/// readable rather than digested so the manifest's assignment table can be
/// audited by eye; a component merged by an unlucky `:` in a path is
/// conservative, since merging can only reduce leakage.
pub fn content_group(identity: &CandidateIdentity) -> String {
    match identity {
        CandidateIdentity::Memory(m) => format!("memory:{}", m.memory_id),
        CandidateIdentity::Code(c) => format!("code:{}:{}", c.repo, c.path),
        CandidateIdentity::Document(d) => format!("document:{}", d.path),
    }
}

/// Assign every row to a component and every component to a split.
pub fn plan(rows: &[GroupInput<'_>], cfg: &SplitConfig) -> SplitPlan {
    let (mut graph, roots) = build(rows);
    let components = components(&mut graph, rows, &roots, cfg);
    let ids: BTreeMap<usize, String> = components
        .iter()
        .map(|(root, component)| (*root, component.id()))
        .collect();

    let mut assignments: Vec<GroupAssignment> = components
        .iter()
        .map(|(root, component)| {
            component.assignment(ids.get(root).cloned().unwrap_or_default(), cfg)
        })
        .collect();
    assignments.sort_by(|a, b| a.group_id.cmp(&b.group_id));

    let by_id: BTreeMap<&str, Split> = assignments
        .iter()
        .map(|a| (a.group_id.as_str(), a.split))
        .collect();
    let per_row = roots
        .iter()
        .map(|root| {
            let id = ids.get(&graph.find(*root)).cloned().unwrap_or_default();
            let split = by_id.get(id.as_str()).copied().unwrap_or(Split::Train);
            (id, split)
        })
        .collect();

    SplitPlan {
        per_row,
        assignments,
    }
}

/// Intern every row's two nodes, union them, and return each row's root.
fn build(rows: &[GroupInput<'_>]) -> (Graph, Vec<usize>) {
    let mut graph = Graph::default();
    let roots = rows
        .iter()
        .map(|row| {
            let q = graph.node(&format!("q\u{0}{}", row.query_group));
            let c = graph.node(&format!("c\u{0}{}", row.content_group));
            graph.union(q, c)
        })
        .collect();
    (graph, roots)
}

/// Every connected component, with its member keys, its row count and the
/// strongest reservation rule any of its rows triggered.
fn components(
    graph: &mut Graph,
    rows: &[GroupInput<'_>],
    roots: &[usize],
    cfg: &SplitConfig,
) -> BTreeMap<usize, Component> {
    let mut components: BTreeMap<usize, Component> = BTreeMap::new();
    for index in 0..graph.parent.len() {
        let root = graph.find(index);
        let key = graph.keys.get(index).cloned().unwrap_or_default();
        components.entry(root).or_default().members.push(key);
    }
    for (row, root) in rows.iter().zip(roots) {
        let root = graph.find(*root);
        let entry = components.entry(root).or_default();
        entry.rows = entry.rows.saturating_add(1);
        entry.forced = stronger(entry.forced.take(), forced_by(row, cfg));
    }
    components
}

/// One connected component while it is being accumulated.
#[derive(Default)]
struct Component {
    members: Vec<String>,
    rows: usize,
    forced: Option<&'static str>,
}

impl Component {
    /// `g-<16hex>` over the component's sorted member keys: independent of the
    /// order rows arrived in.
    fn id(&self) -> String {
        let mut members = self.members.clone();
        members.sort_unstable();
        let digest = sha256_hex(members.join("\n").as_bytes());
        format!("g-{}", digest.get(..ID_HEX).unwrap_or(digest.as_str()))
    }

    /// This component's report row, with its split resolved.
    fn assignment(&self, group_id: String, cfg: &SplitConfig) -> GroupAssignment {
        let split = match self.forced {
            Some(_) => Split::Holdout,
            None => bucket(&cfg.seed, &group_id, cfg.ratios),
        };
        GroupAssignment {
            group_id,
            split,
            forced: self.forced.map(str::to_string),
            queries: self.members.iter().filter(|m| m.starts_with('q')).count(),
            contents: self.members.iter().filter(|m| m.starts_with('c')).count(),
            rows: self.rows,
        }
    }
}

/// Which reservation rule, if any, this row triggers. `repo` outranks `since`
/// so the answer does not depend on which row was seen first.
fn forced_by(row: &GroupInput<'_>, cfg: &SplitConfig) -> Option<&'static str> {
    if let (Some(wanted), Some(repo)) = (cfg.holdout_repo.as_deref(), row.repo)
        && wanted == repo
    {
        return Some("repo");
    }
    match cfg.holdout_since.as_deref() {
        Some(since) if row.at >= since => Some("since"),
        _ => None,
    }
}

/// The stronger of two reservation verdicts.
fn stronger(a: Option<&'static str>, b: Option<&'static str>) -> Option<&'static str> {
    match (a, b) {
        (Some("repo"), _) | (_, Some("repo")) => Some("repo"),
        (Some(reason), _) | (_, Some(reason)) => Some(reason),
        (None, None) => None,
    }
}

/// The split a group id falls into under `seed` and `ratios`.
///
/// The top 53 bits of `sha256(seed || NUL || group_id)` become a value in
/// `[0, 1)` that is exactly representable, so the comparison against the
/// cumulative ratios is the same on every platform.
fn bucket(seed: &str, group_id: &str, ratios: [f64; 3]) -> Split {
    let mut salted = Vec::with_capacity(seed.len() + group_id.len() + 1);
    salted.extend_from_slice(seed.as_bytes());
    salted.push(0);
    salted.extend_from_slice(group_id.as_bytes());
    let digest = sha256_hex(&salted);
    let bits = u64::from_str_radix(digest.get(..ID_HEX).unwrap_or("0"), 16).unwrap_or(0);
    let unit = (bits >> 11) as f64 / (1_u64 << 53) as f64;
    if unit < ratios[0] {
        Split::Train
    } else if unit < ratios[0] + ratios[1] {
        Split::Validation
    } else {
        Split::Holdout
    }
}

/// A union-find over the query and content node keys.
#[derive(Default)]
struct Graph {
    keys: Vec<String>,
    index: BTreeMap<String, usize>,
    parent: Vec<usize>,
}

impl Graph {
    /// The index of `key`, interning it on first sight.
    fn node(&mut self, key: &str) -> usize {
        if let Some(found) = self.index.get(key) {
            return *found;
        }
        let next = self.keys.len();
        self.keys.push(key.to_string());
        self.index.insert(key.to_string(), next);
        self.parent.push(next);
        next
    }

    /// The representative of `index`, with path compression.
    fn find(&mut self, index: usize) -> usize {
        let mut root = index;
        while self.parent.get(root).copied().unwrap_or(root) != root {
            root = self.parent.get(root).copied().unwrap_or(root);
        }
        let mut walk = index;
        while walk != root {
            let next = self.parent.get(walk).copied().unwrap_or(root);
            if let Some(slot) = self.parent.get_mut(walk) {
                *slot = root;
            }
            walk = next;
        }
        root
    }

    /// Merge the two components and return the surviving root.
    fn union(&mut self, a: usize, b: usize) -> usize {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return ra;
        }
        let (keep, drop) = if ra < rb { (ra, rb) } else { (rb, ra) };
        if let Some(slot) = self.parent.get_mut(drop) {
            *slot = keep;
        }
        keep
    }
}

#[cfg(test)]
#[path = "tests/dataset_split.rs"]
mod tests;
