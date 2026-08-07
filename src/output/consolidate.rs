//! Output helpers for `comemory consolidate`. JSON mode serializes the
//! [`Report`] directly; TTY mode prints a scan header, one block per
//! cluster with the keeper marked, and the shared pagination footer.
//! `Report` also doubles as `api::consolidate::run`'s return type — the
//! CLI's `--json` stdout and the `/api/v1/consolidate` response `data`
//! field build from the same owned value.

use std::fmt::Write as _;
use std::io::Write as _;

use serde::Serialize;

use crate::consolidate::{Cluster, Member};
use crate::output::page::Page;
use crate::output::{json, tty};
use crate::prelude::*;

/// The report `comemory consolidate` emits, in JSON and TTY alike.
#[derive(Debug, Serialize)]
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
    pub clusters: Page<Cluster>,
}

/// Members rendered per cluster before the tail takes over. A wide `--radius`
/// legitimately unions most of the corpus into one cluster, and paging is
/// per-cluster, so an uncapped TTY block could print thousands of lines.
/// `--json` is never capped.
const MAX_TTY_MEMBERS: usize = 20;

/// Render `report` to stdout in either JSON or TTY mode.
pub fn emit(report: &Report, json_flag: bool) -> Result<()> {
    if json_flag {
        json::write(report)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    let total = report.clusters.total.unwrap_or(report.clusters.items.len());
    writeln!(
        out,
        "clusters : {total}  ({} memories scanned, radius {})",
        report.scanned, report.radius
    )?;
    if report.skipped_unhashed > 0 {
        writeln!(
            out,
            "  note: {} memories have no fingerprint yet — run `comemory rebuild`",
            report.skipped_unhashed
        )?;
    }
    for (n, cluster) in report.clusters.items.iter().enumerate() {
        write_cluster(&mut out, report.clusters.offset + n + 1, cluster)?;
    }
    tty::write_page_footer(
        &mut out,
        report.clusters.items.len(),
        report.clusters.offset,
        report.clusters.total,
    )
}

/// One cluster block: its header, the capped member lines, and the merge hint.
fn write_cluster(out: &mut impl std::io::Write, ordinal: usize, cluster: &Cluster) -> Result<()> {
    let resolved = if cluster.resolved { ", resolved" } else { "" };
    writeln!(
        out,
        "\ncluster {ordinal}  ({} members, max hamming {}{resolved})",
        cluster.members.len(),
        cluster.max_hamming
    )?;
    for (n, member) in cluster.members.iter().take(MAX_TTY_MEMBERS).enumerate() {
        write_member(out, member, n == 0)?;
    }
    if let Some(rest) = cluster.members.len().checked_sub(MAX_TTY_MEMBERS)
        && rest > 0
    {
        writeln!(out, "  … and {rest} more")?;
    }
    write_hint(out, cluster)
}

/// One member line: id, the stats behind its rank, and its supersede state.
fn write_member(out: &mut impl std::io::Write, member: &Member, is_keeper: bool) -> Result<()> {
    let mut line = format!(
        "  {}  quality {}  accessed {}",
        member.id, member.quality, member.access_count
    );
    if is_keeper {
        line.push_str("  ← keeper");
    } else {
        let _ = write!(line, "  hamming {}", member.hamming_to_keeper);
    }
    if let Some(by) = &member.superseded_by {
        let _ = write!(line, "  superseded by {by}");
    }
    writeln!(out, "{line}")?;
    Ok(())
}

/// The merge hint: the exact `--supersedes` flags that retire the non-keepers.
///
/// A resolved cluster gets none. Its surviving non-keepers are the memories
/// that *did* the superseding, so proposing to retire them would invert the
/// operator's own decision; the cluster is only listed at all under `--all`.
fn write_hint(out: &mut impl std::io::Write, cluster: &Cluster) -> Result<()> {
    if cluster.resolved {
        return Ok(());
    }
    let flags: Vec<String> = cluster
        .members
        .iter()
        .skip(1)
        .filter(|m| m.superseded_by.is_none())
        .map(|m| format!("--supersedes {}", m.id))
        .collect();
    if flags.is_empty() {
        return Ok(());
    }
    writeln!(
        out,
        "\n  merge with: comemory save \"<merged body>\" {}",
        flags.join(" ")
    )?;
    Ok(())
}
