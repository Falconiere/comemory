# consolidate/

**What belongs here:** the read-only advisory near-duplicate cluster report
behind `comemory consolidate` — transitive union-find grouping of live
memories by SimHash Hamming distance, plus keeper-ordering and in-cluster
supersede resolution.

**What does NOT belong here:** the actual merge. Consolidate only reports;
resolving a cluster stays a human act (`comemory save --supersedes <id>`), and
nothing in this module writes to the store.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `cluster.rs` | `Group` | Union-find grouping of live fingerprints into near-duplicate clusters within a Hamming radius |
| `keeper.rs` | `build` | Keeper ordering (quality → access → recency → PageRank → id), member metadata, and in-cluster supersede resolution |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/consolidate.rs` (`pub mod
<name>;`) and callers import concrete paths.
