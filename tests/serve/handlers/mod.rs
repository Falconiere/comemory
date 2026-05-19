//! Test submodules mirroring `src/serve/handlers/`.

#[path = "../../common/graph_fixture.rs"]
mod graph_fixture;

pub mod expand;
pub mod node;
pub mod search;
pub mod seed;
