//! Union-find grouping of live fingerprints into near-duplicate clusters.
//!
//! Transitive on purpose: A—B and B—C put A and C in one group even when
//! A/C exceed the radius. Cliques would keep groups tighter but place one
//! memory in several overlapping clusters, which turns "which one do I keep"
//! into an ambiguous question. Chaining is the accepted cost, made visible by
//! [`Group::max_hamming`].

use crate::simhash::hamming64;
use crate::store::simhash_scan::SimhashRow;

/// One near-duplicate group: its rows plus the widest distance inside it.
#[derive(Debug, Clone)]
pub struct Group {
    /// Member rows, id-ascending.
    pub rows: Vec<SimhashRow>,
    /// Widest real pairwise distance among the members.
    pub max_hamming: u32,
}

/// Grouping outcome plus the counts the report header needs.
#[derive(Debug, Clone)]
pub struct Grouping {
    /// Groups of two or more, in report order.
    pub groups: Vec<Group>,
    /// Rows that carried a real fingerprint and were compared.
    pub scanned: usize,
    /// Rows dropped because their `simhash` was never backfilled.
    pub skipped_unhashed: usize,
}

/// Disjoint-set forest over scan positions, union by ascending root so the
/// grouping is a pure function of the id-ordered input.
struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    /// One singleton set per position.
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    /// Representative of `i`'s set, with path compression.
    fn find(&mut self, i: usize) -> usize {
        let mut root = i;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut cur = i;
        while self.parent[cur] != root {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }

    /// Merge the two sets, keeping the smaller root.
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        self.parent[hi] = lo;
    }
}

/// Group `rows` into near-duplicate clusters of two or more.
///
/// Rows whose `simhash` is `0` are an absent fingerprint, not a real one:
/// clustering them would exact-collide every unbackfilled row into one
/// fabricated mega-cluster, so they are dropped and counted instead.
///
/// The pairwise sweep is O(n²) XOR+popcount — ~50M ops at 10k memories, tens
/// of milliseconds — so no LSH banding or spatial index is warranted at
/// personal-memory scale, and a report command can afford the exact answer.
pub fn group_near_dups(rows: &[SimhashRow], radius: u32) -> Grouping {
    let hashed: Vec<&SimhashRow> = rows.iter().filter(|r| r.simhash != 0).collect();
    let mut uf = UnionFind::new(hashed.len());
    for i in 0..hashed.len() {
        for j in (i + 1)..hashed.len() {
            if distance(hashed[i], hashed[j]) <= radius {
                uf.union(i, j);
            }
        }
    }
    let mut groups = collect_groups(&hashed, &mut uf);
    sort_groups(&mut groups);
    Grouping {
        groups,
        scanned: hashed.len(),
        skipped_unhashed: rows.len() - hashed.len(),
    }
}

/// Hamming distance between two stored fingerprints. The `as u64` cast is the
/// exact inverse of the write side's `of_body(body) as i64`.
fn distance(a: &SimhashRow, b: &SimhashRow) -> u32 {
    hamming64(a.simhash as u64, b.simhash as u64)
}

/// Materialize the sets of size two or more, each with its widest distance.
fn collect_groups(hashed: &[&SimhashRow], uf: &mut UnionFind) -> Vec<Group> {
    let mut by_root: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for i in 0..hashed.len() {
        let root = uf.find(i);
        by_root.entry(root).or_default().push(i);
    }
    by_root
        .into_values()
        .filter(|members| members.len() > 1)
        .map(|members| Group {
            max_hamming: widest(hashed, &members),
            rows: members.iter().map(|&i| hashed[i].clone()).collect(),
        })
        .collect()
}

/// Widest pairwise distance among `members` — the chaining tell.
fn widest(hashed: &[&SimhashRow], members: &[usize]) -> u32 {
    let mut max = 0;
    for (n, &i) in members.iter().enumerate() {
        for &j in &members[(n + 1)..] {
            max = max.max(distance(hashed[i], hashed[j]));
        }
    }
    max
}

/// Report order: biggest clusters first, tightest among equals, then id.
fn sort_groups(groups: &mut [Group]) {
    groups.sort_by(|a, b| {
        b.rows
            .len()
            .cmp(&a.rows.len())
            .then(a.max_hamming.cmp(&b.max_hamming))
            .then_with(|| first_id(a).cmp(first_id(b)))
    });
}

/// Lowest id in the group — the total-order tiebreak for report position.
fn first_id(group: &Group) -> &str {
    group
        .rows
        .first()
        .map_or("", |row: &SimhashRow| row.id.as_str())
}

#[cfg(test)]
#[path = "tests/cluster.rs"]
mod tests;
