//! What a benchmark run was measured on, and what it was measured against:
//! hardware, memory use, the run-scoped working set, and the pinned
//! corpus/index snapshot behind [`RetrievalVersion`].
//!
//! Process RSS comes from Linux's own `/proc/self/status`; every other
//! platform reports `None` rather than pulling in a system-introspection
//! dependency, and the portable `observation_bytes` figure is always present.

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::domains::learning::evaluation::candidate_observation::{
    CorpusSnapshot, RepoRevision, RetrievalKnobs, RetrievalVersion, canonical_digest,
};
use crate::domains::retrieval::code_rerank::WorkingSet;
use crate::prelude::*;
use crate::store::stats_counts::Corpus;
use crate::store::{Connection, repos_inventory, stats_counts};

/// Where a benchmark run happened, and what it cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEnvironment {
    /// Target OS the binary was built for.
    pub os: String,
    /// Target architecture.
    pub arch: String,
    /// Logical CPUs available to the process; `0` when unavailable.
    pub cpus: usize,
    /// Peak resident set size in bytes, where the platform exposes it.
    pub peak_rss_bytes: Option<u64>,
    /// Bytes of candidate text every observation in the run holds.
    pub observation_bytes: usize,
    /// The largest single task's share of `observation_bytes`.
    pub peak_task_observation_bytes: usize,
    /// Repo label the process CWD resolved to when the run started, if any.
    /// Run-scoped and indicative: the code leg builds its own working set per
    /// query, from this same CWD, and does not hand it back.
    pub working_set_repo: Option<String>,
    /// Files in that working set — the one machine-dependent input to code
    /// ranking, recorded so a cross-machine difference is visible.
    pub working_set_files: usize,
}

impl RunEnvironment {
    /// Capture the hardware and working-set facts. Text sizes are filled in by
    /// [`RunEnvironment::record_observation_bytes`] once the tasks have run.
    pub fn capture() -> Self {
        let working_set = WorkingSet::from_cwd(None);
        let files = working_set.files();
        RunEnvironment {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            cpus: std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
            peak_rss_bytes: peak_rss_bytes(),
            observation_bytes: 0,
            peak_task_observation_bytes: 0,
            working_set_repo: files.first().and_then(|id| repo_of(id)),
            working_set_files: files.len(),
        }
    }

    /// Fold one task's candidate-text footprint into the totals, and re-read
    /// peak RSS so the figure covers the whole run rather than its start.
    pub fn record_observation_bytes(&mut self, task_bytes: usize) {
        self.observation_bytes = self.observation_bytes.saturating_add(task_bytes);
        self.peak_task_observation_bytes = self.peak_task_observation_bytes.max(task_bytes);
        self.peak_rss_bytes = peak_rss_bytes().or(self.peak_rss_bytes);
    }
}

/// The repo label inside a `file:<repo>:<path>` working-set id.
fn repo_of(file_id: &str) -> Option<String> {
    let mut parts = file_id.splitn(3, ':');
    match (parts.next(), parts.next()) {
        (Some("file"), Some(repo)) if !repo.is_empty() => Some(repo.to_string()),
        _ => None,
    }
}

/// Peak resident set size from `/proc/self/status`'s `VmHWM`, in bytes.
/// `None` wherever that file does not exist, which is every non-Linux target.
fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// Snapshot which comemory, which schema, which knobs and which corpus this run
/// scores against. Two runs with equal `knobs_hash` used the same ranking
/// configuration; two with equal `corpus.digest` read the same index revision.
pub fn retrieval_version(cfg: &Config, conn: &Connection) -> Result<RetrievalVersion> {
    let knobs = RetrievalKnobs::of(cfg);
    let knobs_hash = canonical_digest(&knobs)?;
    Ok(RetrievalVersion {
        binary_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version: crate::store::migrate::CURRENT_VERSION.to_string(),
        knobs,
        knobs_hash,
        corpus: corpus_snapshot(conn)?,
    })
}

/// The pinned corpus / index snapshot: live row counts plus every indexed
/// repository's HEAD at its last index, digested into one snapshot id.
fn corpus_snapshot(conn: &Connection) -> Result<CorpusSnapshot> {
    let repos: Vec<RepoRevision> = repos_inventory::fetch(conn, None)?
        .into_iter()
        .map(|row| RepoRevision {
            repo: row.repo,
            last_head: row.last_head,
            last_indexed_at: row.last_indexed_at,
        })
        .collect();
    let mut snapshot = CorpusSnapshot {
        memories: stats_counts::scoped_count(conn, Corpus::LiveMemories, None)?,
        code_symbols: stats_counts::scoped_count(conn, Corpus::CodeSymbols, None)?,
        documents: stats_counts::scoped_count(conn, Corpus::Documents, None)?,
        document_chunks: stats_counts::count_table(conn, "document_chunks")?,
        repos,
        digest: String::new(),
    };
    snapshot.digest = canonical_digest(&snapshot)?;
    Ok(snapshot)
}

#[cfg(test)]
#[path = "tests/run_environment.rs"]
mod tests;
