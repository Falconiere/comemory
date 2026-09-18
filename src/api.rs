//! The emptied command-core shell.
//!
//! `api::<cmd>::run(&mut Ctx, Request)` once held the logic `cli::` and
//! `serve::routes::` both call. Every core has moved under `domains::`:
//! [#175](https://github.com/Falconiere/comemory/issues/175) took the last two,
//! the integrations cores, into `crate::domains::integrations`. Nothing is
//! declared here any more, and
//! [#178](https://github.com/Falconiere/comemory/issues/178) removes the module
//! itself with the rest of the legacy layer. The execution context is
//! transport-neutral and lives in [`crate::utilities::context::Ctx`].
