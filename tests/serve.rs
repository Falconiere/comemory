//! Test binary for the `serve` module. Each submodule mirrors a file
//! under `src/serve/`.

#[path = "serve/state.rs"]
mod state;

#[path = "serve/dto.rs"]
mod dto;

#[path = "serve/error.rs"]
mod error;

#[path = "serve/assets.rs"]
mod assets;
