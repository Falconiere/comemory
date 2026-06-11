// Shared real-git fixture helpers, declared ONCE at the binary root so
// `cochange` and `materialize` can both `use crate::git_setup` without
// tripping clippy::duplicate_mod.
#[path = "common/git_setup.rs"]
mod git_setup;

#[path = "graph/cochange.rs"]
mod cochange;

#[path = "graph/cross_link.rs"]
mod cross_link;

#[path = "graph/edges.rs"]
mod edges;

#[path = "graph/imports.rs"]
mod imports;

#[path = "graph/materialize.rs"]
mod materialize;

#[path = "graph/pagerank.rs"]
mod pagerank;
