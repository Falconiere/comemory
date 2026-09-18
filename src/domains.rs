//! Business capabilities.
//!
//! Each capability owns one area of behavior end to end — its models, its
//! algorithms, and the command cores both delivery adapters call — and is
//! declared here as a sibling module file plus its folder. A capability may
//! depend on `store`, `config`, `errors`, `prelude` and the named shared
//! primitives in [`crate::utilities`]; it must never import `cli`, `serve`,
//! `output`, or the legacy `api` command-core tree.
//!
//! Filled one slice at a time by the [#164 migration][m]; every module here is
//! a real capability with production code, never a placeholder.
//!
//! [m]: https://github.com/Falconiere/comemory/issues/164

/// AST extraction, code indexing, the repository inventory, and Git hooks.
pub mod code;
