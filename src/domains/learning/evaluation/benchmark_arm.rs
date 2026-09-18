//! Benchmark arms: the deterministic baseline and the scored arms an external
//! scorer supplies through a JSON file.
//!
//! An arm never re-runs retrieval. It reorders one already-captured candidate
//! snapshot, which is what makes the comparison paired: every arm sees exactly
//! the same candidates, in the same pool positions, with the same bounded text.
//! No subprocess, no timeout and no wire protocol is defined here — a scorer's
//! output arrives as a file.

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::prelude::*;

/// The reserved name of the deterministic baseline arm.
pub const BASELINE_ARM: &str = "deterministic";

/// One external scorer's output over a benchmark run.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArmScores {
    /// Arm name, unique within one run and never [`BASELINE_ARM`].
    pub arm: String,
    /// Free-form scorer identity, recorded in the artifact.
    #[serde(default)]
    pub scorer_version: Option<String>,
    /// `task id -> candidate ref -> score`. Higher is better.
    pub scores: HashMap<String, HashMap<String, f64>>,
}

impl ArmScores {
    /// Read and validate a scores file against the task ids the set declares.
    ///
    /// A file naming an unknown task or reserving the baseline's name is
    /// refused: a stale scores file half-applied would silently compare an arm
    /// against the wrong candidates.
    pub fn load(path: &Path, task_ids: &[String]) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("scores file {}: {e}", path.display())))?;
        let scores: ArmScores = serde_json::from_str(&raw)
            .map_err(|e| Error::Config(format!("scores file {}: {e}", path.display())))?;
        scores
            .validate(task_ids)
            .map_err(|e| Error::Config(format!("scores file {}: {e}", path.display())))?;
        Ok(scores)
    }

    /// Reject the shapes that would make this arm's comparison meaningless.
    ///
    /// A score JSON cannot represent — `1e400` — is already refused by the
    /// parser, naming the file and the column, so there is no finiteness pass
    /// here: `serde_json` has no NaN or infinity literal to let one through.
    fn validate(&self, task_ids: &[String]) -> Result<()> {
        if self.arm.trim().is_empty() {
            return Err(Error::Config("arm name must not be empty".into()));
        }
        if self.arm == BASELINE_ARM {
            return Err(Error::Config(format!(
                "arm name `{BASELINE_ARM}` is reserved for the baseline"
            )));
        }
        for task in self.scores.keys() {
            if !task_ids.iter().any(|id| id == task) {
                return Err(Error::Config(format!(
                    "task `{task}` is not in this benchmark set"
                )));
            }
        }
        Ok(())
    }

    /// This arm's scores for `task`, or an empty map when it scored none.
    pub fn for_task(&self, task: &str) -> Option<&HashMap<String, f64>> {
        self.scores.get(task)
    }
}

/// One arm's ordering of one task's pool, plus how much of it the arm scored.
#[derive(Debug, Clone)]
pub struct ArmOrder {
    /// 1-based pool positions in the arm's order.
    pub order: Vec<usize>,
    /// Share of the pool the arm supplied a score for.
    pub scored_fraction: f64,
}

/// The identity ordering: the pool exactly as retrieval produced it.
pub fn baseline_order(pool_size: usize) -> ArmOrder {
    ArmOrder {
        order: (1..=pool_size).collect(),
        scored_fraction: 1.0,
    }
}

/// Reorder `candidate_refs` (indexed by `pool_position - 1`) by `scores`.
///
/// Scored candidates come first, descending by score, ties broken by ascending
/// pool position so the order is total. Unscored candidates keep their relative
/// pool order and follow, which makes a partially-scored arm degrade toward the
/// baseline instead of shuffling what it never judged. An arm that scored none
/// of this task's candidates therefore reproduces [`baseline_order`] exactly.
pub fn scored_order<S: BuildHasher>(
    candidate_refs: &[String],
    scores: &HashMap<String, f64, S>,
) -> ArmOrder {
    let mut ranked: Vec<(usize, f64)> = Vec::new();
    let mut rest: Vec<usize> = Vec::new();
    for (index, candidate_ref) in candidate_refs.iter().enumerate() {
        match scores.get(candidate_ref) {
            Some(score) => ranked.push((index + 1, *score)),
            None => rest.push(index + 1),
        }
    }
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let scored_count = ranked.len();
    let mut order: Vec<usize> = ranked.into_iter().map(|(position, _)| position).collect();
    order.extend(rest);
    ArmOrder {
        order,
        scored_fraction: if candidate_refs.is_empty() {
            0.0
        } else {
            scored_count as f64 / candidate_refs.len() as f64
        },
    }
}

#[cfg(test)]
#[path = "tests/benchmark_arm.rs"]
mod tests;
