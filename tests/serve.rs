//! Unit tests for the `comemory serve` logic modules, mirroring `src/serve/`
//! 1:1 (less `mod.rs`). Security-critical pure logic — token comparison, the
//! loopback Host guard, and path containment — is tested hardest here; the
//! async glue in `router.rs`/`handlers.rs` is exercised end-to-end by
//! `tests/cli/serve.rs` instead.

#[path = "serve/assets.rs"]
mod assets;

#[path = "serve/error.rs"]
mod error;

#[path = "serve/fileio.rs"]
mod fileio;

#[path = "serve/handlers.rs"]
mod handlers;

#[path = "serve/repo_root.rs"]
mod repo_root;

#[path = "serve/router.rs"]
mod router;

#[path = "serve/security.rs"]
mod security;
