//! `api::eval::{Request, run}` — the shared middle of `comemory eval` /
//! `POST /api/v1/eval`: build the merged golden set (file ∪ feedback
//! harvest) and score the real pipeline against it with tracking off.
//! Moved out of `cli::eval::run` (Binding Rule 1).
//!
//! `eval` mutates nothing (§Route map Notes): the run is read-class even
//! though it can take a while, which is why the HTTP route runs it as a
//! non-mutating job (no write permit, unaffected by `--read-only`).

use std::path::Path;

use serde::Deserialize;

use crate::api::Ctx;
use crate::eval::golden;
use crate::eval::runner::{self, EvalReport};
use crate::prelude::*;

/// `comemory eval` / `POST /api/v1/eval` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Path to a YAML golden file (`- query: ...` / `  relevant: [..]`).
    /// Over HTTP, filesystem containment is enforced by the route handler
    /// BEFORE this runs (§Security "Path containment") — `run` treats it
    /// as an already-safe path.
    #[serde(default)]
    pub golden: Option<String>,
    /// Skip the feedback harvest; use only `golden`.
    #[serde(default)]
    pub golden_only: bool,
    /// recall@k cut.
    #[serde(default = "default_k")]
    pub k: usize,
}

/// The CLI's `--k` default (`GoldenSetArgs`), reused by `tune`/`bandit`'s
/// `Request` too so an HTTP request omitting `k` scores identically to the
/// CLI across all three commands.
pub(crate) fn default_k() -> usize {
    3
}

/// Build the merged golden set and score the real pipeline against it
/// (tracking off — measurement must not feed the signals it measures).
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<EvalReport> {
    let cfg = ctx.cfg;
    let conn = ctx.conn()?;
    let pairs = golden::resolve(
        &*conn,
        req.golden.as_deref().map(Path::new),
        req.golden_only,
    )?;
    runner::run_eval(cfg, &*conn, &pairs, req.k)
}
