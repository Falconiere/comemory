//! `api::consolidate::{Request, run}` — the shared middle of `comemory
//! consolidate` / `GET /api/v1/consolidate`: cluster live memories by
//! SimHash distance within a radius and page the result. Moved out of
//! `cli::consolidate::run` (Binding Rule 1). Read-only end to end — see
//! `crate::consolidate`'s module doc.

use serde::Deserialize;

use crate::api::Ctx;
use crate::cli::page_window;
use crate::consolidate::{self, Options};
use crate::output::consolidate::Report;
use crate::output::page::Page;
use crate::prelude::*;

/// `comemory consolidate` / `GET /api/v1/consolidate` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Hamming radius linking two fingerprints. Defaults to the configured
    /// `rank.near_dup_hamming`; SimHash is 64-bit, so 64 is the ceiling.
    #[serde(default)]
    pub radius: Option<u32>,
    /// Restrict the scan to one repo. Default: every repo.
    #[serde(default)]
    pub repo: Option<String>,
    /// Include clusters already resolved by live supersede edges.
    #[serde(default)]
    pub all: bool,
    /// Page size — overrides the configured `retrieval.top_k`. `0` means
    /// "all remaining".
    #[serde(default)]
    pub k: Option<usize>,
    /// Number of leading clusters to skip (deep paging).
    #[serde(default)]
    pub offset: usize,
}

/// Scan, cluster, and page live memories within `req.radius` (or the
/// configured default).
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Report> {
    let cfg = ctx.cfg;
    let radius = req.radius.unwrap_or(cfg.rank.near_dup_hamming);
    let window = page_window(cfg, req.k, req.offset);
    let conn: &rusqlite::Connection = ctx.conn()?;
    let scan = consolidate::detect(
        conn,
        &Options {
            radius,
            repo: req.repo,
            include_resolved: req.all,
        },
    )?;
    Ok(Report {
        radius,
        scanned: scan.scanned,
        skipped_unhashed: scan.skipped_unhashed,
        clustered: scan.clustered(),
        clusters: Page::from_slice(scan.clusters, window.limit, window.offset),
    })
}
