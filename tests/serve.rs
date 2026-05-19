//! Test binary for the `serve` module. Each submodule mirrors a file
//! under `src/serve/`.

#[path = "common/graph_fixture.rs"]
pub mod graph_fixture;

#[path = "serve/state.rs"]
mod state;

#[path = "serve/dto.rs"]
mod dto;

#[path = "serve/error.rs"]
mod error;

#[path = "serve/assets.rs"]
mod assets;

#[path = "serve/router.rs"]
mod router;

#[path = "serve/handlers/mod.rs"]
mod handlers;
