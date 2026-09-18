//! `domains::learning::benchmark::{Request, run}` — the middle of
//! `comemory benchmark`: load a reviewed, versioned set, capture each task's
//! candidate pool through the real retrieval legs, score every arm over that
//! one snapshot, and return the replayable artifact.
//!
//! Nothing here writes to the database. `unified::run_legs` has no telemetry
//! path at all, so no `retrieval_log` row is written and no access counter is
//! bumped; the set's pinned `ranking.decay` is what freezes ACT-R activation,
//! because disabling access tracking alone would not.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use time::OffsetDateTime;

use crate::domains::learning::evaluation::benchmark_arm::{self, ArmScores};
use crate::domains::learning::evaluation::benchmark_arm_report::ArmInput;
use crate::domains::learning::evaluation::benchmark_report::{
    self, BenchmarkReport, ReportContext,
};
use crate::domains::learning::evaluation::benchmark_runner::{self, RunContext, TaskCapture};
use crate::domains::learning::evaluation::benchmark_set::BenchmarkSet;
use crate::domains::learning::evaluation::run_environment::{self, RunEnvironment};
use crate::prelude::*;
use crate::store::memory_row;
use crate::utilities::context::Ctx;

/// `comemory benchmark` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Path to the reviewed benchmark set YAML.
    pub set: PathBuf,
    /// Zero or more scores files, each adding one arm.
    #[serde(default)]
    pub scores: Vec<PathBuf>,
    /// Override the set's `defaults.k`.
    #[serde(default)]
    pub k: Option<usize>,
}

/// Load the set, capture every task, score every arm, and build the artifact.
pub fn run(ctx: &mut Ctx<'_>, req: &Request) -> Result<BenchmarkReport> {
    let set = BenchmarkSet::load(&req.set)?;
    let k = resolve_k(&set, req.k)?;
    let task_ids: Vec<String> = set.tasks.iter().map(|t| t.id.clone()).collect();
    let arm_scores = load_arms(&req.scores, &task_ids)?;
    let cfg = set.effective_config(ctx.cfg)?;
    let reference_time = memory_row::iso_format(OffsetDateTime::now_utc())?;

    let conn = ctx.conn()?;
    let version = run_environment::retrieval_version(&cfg, conn)?;
    let mut environment = RunEnvironment::capture();
    let shared = RunContext {
        k,
        version: &version,
        reference_time: reference_time.clone(),
    };
    let mut captures = Vec::with_capacity(set.tasks.len());
    for task in &set.tasks {
        let capture = benchmark_runner::capture(&cfg, conn, &set, task, &shared)?;
        environment.record_observation_bytes(observation_bytes(&capture));
        captures.push(capture);
    }

    let arms = build_arms(&captures, &arm_scores);
    Ok(benchmark_report::build(
        &set,
        &captures,
        &arms,
        ReportContext {
            k,
            version,
            reference_time,
            environment,
        },
    ))
}

/// The recall@k / nDCG@k cut: the request's override, else the set's default.
/// A zero cut would score an empty page for every task.
fn resolve_k(set: &BenchmarkSet, override_k: Option<usize>) -> Result<usize> {
    match override_k {
        None => Ok(set.defaults.k),
        Some(0) => Err(Error::Usage("--k must be at least 1".into())),
        Some(k) => Ok(k),
    }
}

/// Load every scores file, refusing a duplicate arm name: two arms sharing a
/// name would make the report's per-task arm map ambiguous.
fn load_arms(paths: &[PathBuf], task_ids: &[String]) -> Result<Vec<ArmScores>> {
    let mut loaded: Vec<ArmScores> = Vec::with_capacity(paths.len());
    for path in paths {
        let scores = ArmScores::load(Path::new(path), task_ids)?;
        if loaded.iter().any(|other| other.arm == scores.arm) {
            return Err(Error::Config(format!(
                "scores file {}: arm `{}` is already declared by another file",
                path.display(),
                scores.arm
            )));
        }
        loaded.push(scores);
    }
    Ok(loaded)
}

/// The baseline arm followed by one arm per scores file, each ordering the same
/// captured pools. No arm re-runs retrieval.
fn build_arms<'a>(captures: &[TaskCapture], scores: &'a [ArmScores]) -> Vec<ArmInput<'a>> {
    let mut arms = vec![ArmInput {
        name: benchmark_arm::BASELINE_ARM.to_string(),
        scores: None,
        orders: captures
            .iter()
            .map(|c| benchmark_arm::baseline_order(c.observation.candidates.len()))
            .collect(),
    }];
    for entry in scores {
        arms.push(ArmInput {
            name: entry.arm.clone(),
            scores: Some(entry),
            orders: captures
                .iter()
                .map(|c| {
                    let refs: Vec<String> = c
                        .observation
                        .candidates
                        .iter()
                        .map(|candidate| candidate.candidate_ref.clone())
                        .collect();
                    entry.for_task(&c.task_id).map_or_else(
                        || benchmark_arm::baseline_order(refs.len()),
                        |scores| benchmark_arm::scored_order(&refs, scores),
                    )
                })
                .collect(),
        });
    }
    arms
}

/// Bytes of candidate text one task's observation holds — the portable memory
/// figure, reported beside the platform's peak RSS where one is available.
fn observation_bytes(capture: &TaskCapture) -> usize {
    capture
        .observation
        .candidates
        .iter()
        .map(|c| c.text.text.len())
        .sum()
}

#[cfg(test)]
#[path = "tests/benchmark.rs"]
mod tests;
