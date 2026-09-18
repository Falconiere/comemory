//! The replayable benchmark artifact: every candidate observation, every arm's
//! result over that one snapshot, and the dataset/environment facts that make
//! the numbers interpretable.
//!
//! `--report` writes it whole; `--json` prints [`BenchmarkReport::summary`],
//! the same shape with every candidate list emptied so it stays pipeable.

use serde::{Deserialize, Serialize};

use crate::domains::learning::evaluation::benchmark_arm_report::{
    self, ArmContext, ArmInput, ArmReport,
};
use crate::domains::learning::evaluation::benchmark_metrics::{self, TaskMetrics};
use crate::domains::learning::evaluation::benchmark_runner::TaskCapture;
use crate::domains::learning::evaluation::benchmark_set::{BenchmarkSet, Budgets};
use crate::domains::learning::evaluation::candidate_observation::{
    OBSERVATION_VERSION, QueryObservation, RetrievalVersion,
};
use crate::domains::learning::evaluation::run_environment::RunEnvironment;

/// Version of the artifact envelope. Distinct from the observation contract's
/// own version, which the envelope also carries.
pub const ARTIFACT_VERSION: u32 = 1;

/// The size of the dataset a run scored.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SetSize {
    /// Schema version of the set file.
    pub version: u32,
    /// Tasks declared.
    pub tasks: usize,
    /// Judgments declared across every task.
    pub judgments: usize,
}

/// One task's row in the artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskReport {
    /// The task this row belongs to.
    pub task_id: String,
    /// The candidate observation, with every candidate.
    pub observation: QueryObservation,
    /// Whether the captured pool's prefix equals the production page.
    pub page_matches_production: bool,
    /// Judgments that matched at least one pooled candidate.
    pub judgments_matched: usize,
    /// Judgments that matched nothing retrieval produced.
    pub judgments_unmatched: usize,
    /// Judgments excluded because a pinned content version no longer holds.
    pub judgments_stale: usize,
    /// Targets of the unmatched judgments; a missing positive is reported,
    /// never inserted into the pool.
    pub unmatched_targets: Vec<String>,
    /// Candidates whose row vanished before their text could be read.
    pub text_unavailable: usize,
    /// Each arm's metrics on this task, keyed by arm name.
    pub arms: Vec<(String, TaskMetrics)>,
}

/// The whole replayable artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    /// [`ARTIFACT_VERSION`].
    pub artifact_version: u32,
    /// [`OBSERVATION_VERSION`] every observation below was written at.
    pub observation_version: u32,
    /// The set's name.
    pub set_name: String,
    /// The dataset's size.
    pub set_size: SetSize,
    /// Where the run happened and what it cost.
    pub environment: RunEnvironment,
    /// What it scored against.
    pub retrieval: RetrievalVersion,
    /// RFC 3339 UTC instant the run started.
    pub reference_time: String,
    /// Whether ACT-R activation was time-independent for this run.
    pub decay_frozen: bool,
    /// The recall@k / nDCG@k cut.
    pub k: usize,
    /// The budgets every verdict was read against.
    pub budgets: Budgets,
    /// Every arm, baseline first.
    pub arms: Vec<ArmReport>,
    /// Every task, in set order.
    pub tasks: Vec<TaskReport>,
}

impl BenchmarkReport {
    /// A copy with every candidate list emptied: the summary `--json` prints,
    /// small enough to pipe while carrying every other field verbatim.
    #[must_use]
    pub fn summary(&self) -> BenchmarkReport {
        let mut summary = self.clone();
        for task in &mut summary.tasks {
            task.observation.candidates.clear();
        }
        summary
    }
}

/// The run-wide facts [`build`] needs that are neither the set nor the
/// captures.
pub struct ReportContext {
    /// The recall@k / nDCG@k cut.
    pub k: usize,
    /// What the run scored against.
    pub version: RetrievalVersion,
    /// RFC 3339 UTC instant the run started.
    pub reference_time: String,
    /// Where the run happened and what it cost.
    pub environment: RunEnvironment,
}

/// Assemble the artifact from the captures and the scored arms. `arms[0]` is
/// the baseline every other arm's paired delta is taken against.
pub fn build(
    set: &BenchmarkSet,
    captures: &[TaskCapture],
    arms: &[ArmInput<'_>],
    context: ReportContext,
) -> BenchmarkReport {
    let baseline_ndcg = baseline_ndcg(captures, arms.first(), context.k);
    let arm_reports: Vec<ArmReport> = arms
        .iter()
        .enumerate()
        .map(|(index, arm)| {
            benchmark_arm_report::build(
                captures,
                arm,
                &ArmContext {
                    k: context.k,
                    is_baseline: index == 0,
                    baseline_ndcg: &baseline_ndcg,
                    budgets: set.budgets,
                },
            )
        })
        .collect();
    BenchmarkReport {
        artifact_version: ARTIFACT_VERSION,
        observation_version: OBSERVATION_VERSION,
        set_name: set.name.clone(),
        set_size: SetSize {
            version: set.version,
            tasks: set.tasks.len(),
            judgments: set.tasks.iter().map(|t| t.judgments.len()).sum(),
        },
        environment: context.environment,
        decay_frozen: context.version.knobs.decay_frozen(),
        retrieval: context.version,
        reference_time: context.reference_time,
        k: context.k,
        budgets: set.budgets,
        tasks: task_reports(captures, arms, context.k),
        arms: arm_reports,
    }
}

/// The baseline arm's per-task nDCG@k, the reference every paired delta uses.
fn baseline_ndcg(captures: &[TaskCapture], arm: Option<&ArmInput<'_>>, k: usize) -> Vec<f64> {
    match arm {
        Some(arm) => benchmark_arm_report::score_arm(captures, arm, k, None)
            .into_iter()
            .map(|m| m.ndcg_at_k)
            .collect(),
        None => vec![0.0; captures.len()],
    }
}

/// One row per task, carrying its observation and every arm's metrics on it.
fn task_reports(captures: &[TaskCapture], arms: &[ArmInput<'_>], k: usize) -> Vec<TaskReport> {
    let mut rows = Vec::with_capacity(captures.len());
    for (index, capture) in captures.iter().enumerate() {
        let matched = capture
            .matched
            .iter()
            .filter(|j| !j.pool_positions.is_empty())
            .count();
        rows.push(TaskReport {
            task_id: capture.task_id.clone(),
            observation: capture.observation.clone(),
            page_matches_production: capture.page_matches_production,
            judgments_matched: matched,
            judgments_unmatched: capture.unmatched_targets.len(),
            judgments_stale: capture.stale,
            unmatched_targets: capture.unmatched_targets.clone(),
            text_unavailable: capture.text_unavailable,
            arms: task_arm_metrics(capture, arms, index, k),
        });
    }
    rows
}

/// Every arm's metrics on one task, in arm order.
fn task_arm_metrics(
    capture: &TaskCapture,
    arms: &[ArmInput<'_>],
    index: usize,
    k: usize,
) -> Vec<(String, TaskMetrics)> {
    arms.iter()
        .map(|arm| {
            let order = arm.orders.get(index).map(|o| o.order.as_slice());
            let metrics =
                benchmark_metrics::score(&capture.matched, order.unwrap_or_default(), k, None);
            (arm.name.clone(), metrics)
        })
        .collect()
}
