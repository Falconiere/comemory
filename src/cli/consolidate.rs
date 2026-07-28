//! `comemory consolidate` — advisory near-duplicate cluster report.
//!
//! Read-only: it groups live memories by SimHash distance and names the
//! member worth keeping, then stops. Merging stays a human act
//! (`comemory save … --supersedes <id>`), so nothing here writes to the
//! store, the edges, or the markdown.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::{load_config, page_window, resolve_data_dir};
use crate::config::paths::Paths;
use crate::consolidate::{self, Options};
use crate::output;
use crate::output::page::Page;
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Which memories say nearly the same thing?
  comemory consolidate

  # Machine-readable clusters plus the page cursor
  comemory consolidate --json

  # Cast a wider net, one repo only
  comemory consolidate --radius 12 --repo comemory

  # Include clusters already settled with supersede edges
  comemory consolidate --all

Clusters are transitive: A near B and B near C puts all three together even
when A and C are further apart than the radius, so `max_hamming` is reported
per cluster. The keeper is the member the ranker already favours — highest
quality, then most retrieved, then most recently used, then highest PageRank.";

/// Arguments to `comemory consolidate`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Hamming radius linking two fingerprints. Defaults to the configured
    /// `rank.near_dup_hamming`; SimHash is 64-bit, so 64 is the ceiling.
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=64))]
    pub radius: Option<u32>,
    /// Restrict the scan to one repo. Default: every repo.
    #[arg(long)]
    pub repo: Option<String>,
    /// Include clusters already resolved by live supersede edges.
    #[arg(long)]
    pub all: bool,
    /// Page size — overrides the configured `retrieval.top_k`. `--limit` is
    /// an accepted alias. `0` means "all remaining".
    #[arg(long, visible_alias = "limit")]
    pub k: Option<usize>,
    /// Number of leading clusters to skip (deep paging).
    #[arg(long, default_value_t = 0)]
    pub offset: usize,
}

/// The report `comemory consolidate` emits, in JSON and TTY alike.
#[derive(Debug, serde::Serialize)]
pub struct Report {
    /// Radius the clusters were built at.
    pub radius: u32,
    /// Live memories carrying a real fingerprint that were compared.
    pub scanned: usize,
    /// Live rows skipped because their `simhash` was never backfilled.
    pub skipped_unhashed: usize,
    /// Memories that landed in a reported cluster.
    pub clustered: usize,
    /// The windowed clusters.
    pub clusters: Page<consolidate::Cluster>,
}

/// Run `comemory consolidate`: scan, cluster, page, emit. Exit code is 0
/// whether or not duplication was found — the report is advisory, not a gate.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;

    let radius = a.radius.unwrap_or(cfg.rank.near_dup_hamming);
    let scan = consolidate::detect(
        &conn,
        &Options {
            radius,
            repo: a.repo.clone(),
            include_resolved: a.all,
        },
    )?;

    let window = page_window(&cfg, a.k, a.offset);
    let report = Report {
        radius,
        scanned: scan.scanned,
        skipped_unhashed: scan.skipped_unhashed,
        clustered: scan.clustered(),
        clusters: Page::from_slice(scan.clusters, window.limit, window.offset),
    };
    output::consolidate::emit(&report, json_flag)
}
