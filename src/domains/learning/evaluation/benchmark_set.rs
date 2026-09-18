//! The versioned benchmark set: reviewed tasks, their per-domain filters, the
//! pinned retrieval configuration, and the budgets a verdict is read against.
//!
//! Every filter narrows specific legs, so a task that sets a filter belonging
//! to a domain it is not scoped to fails to load rather than running with a
//! silently inert narrowing. Lexical and BYO-vector are separate scenarios and
//! a set declares exactly one of them.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::domains::learning::evaluation::judgment::{Judgment, MAX_RELEVANCE, TargetKey};
use crate::domains::learning::evaluation::task_filters::{
    TaskFilters, domain_of, validate_task_filters,
};
use crate::domains::learning::evaluation::tune::{self, TuneCandidate};
use crate::domains::retrieval::scope::{Domain, Domains};
use crate::prelude::*;

/// The only benchmark-set schema version this binary reads.
pub const SET_VERSION: u32 = 1;

/// Which legs a task runs, and therefore which filters it may set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskDomain {
    /// The memory leg only.
    Memory,
    /// The code leg only.
    Code,
    /// The document leg only.
    Document,
    /// Every leg, fused — the mixed-domain case.
    #[default]
    All,
}

impl TaskDomain {
    /// The retrieval domain mask for this scope.
    pub fn mask(self) -> Domains {
        match self {
            TaskDomain::Memory => Domains::of(&[Domain::Memory]),
            TaskDomain::Code => Domains::of(&[Domain::Code]),
            TaskDomain::Document => Domains::of(&[Domain::Document]),
            TaskDomain::All => Domains::all(),
        }
    }

    /// Its wire spelling, for error messages and the artifact.
    pub fn as_str(self) -> &'static str {
        match self {
            TaskDomain::Memory => "memory",
            TaskDomain::Code => "code",
            TaskDomain::Document => "document",
            TaskDomain::All => "all",
        }
    }
}

/// One reviewed task: a query, its filters, and its judgments.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkTask {
    /// Stable task id; keys the per-task report and every scores file.
    pub id: String,
    /// Which legs this task runs.
    #[serde(default)]
    pub domain: TaskDomain,
    /// The query text, run verbatim.
    pub query: String,
    /// The per-domain narrowing.
    #[serde(default)]
    pub filters: TaskFilters,
    /// The reproducible supplied embedding, in a BYO-vector set.
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    /// The reviewed judgments for this query.
    pub judgments: Vec<Judgment>,
}

impl BenchmarkTask {
    /// Every judgment's validated target key, in declaration order.
    pub fn target_keys(&self) -> Result<Vec<TargetKey>> {
        self.judgments
            .iter()
            .map(|j| j.target.resolve(&self.id))
            .collect()
    }
}

/// The embedder identity a BYO-vector set's vectors came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorSpec {
    /// Free-form embedder identity, e.g. `ollama:nomic-embed-text`.
    pub model: String,
    /// Required length of every task's vector.
    pub dim: usize,
}

/// Set-wide defaults a task does not override.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    /// The recall@k / nDCG@k cut.
    #[serde(default = "default_k")]
    pub k: usize,
    /// The `BoundedText` byte bound.
    #[serde(default = "default_max_text_bytes")]
    pub max_text_bytes: usize,
}

/// The default recall@k / nDCG@k cut.
fn default_k() -> usize {
    5
}

/// The default candidate-text byte bound.
fn default_max_text_bytes() -> usize {
    4096
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults {
            k: default_k(),
            max_text_bytes: default_max_text_bytes(),
        }
    }
}

/// The thresholds a verdict is read against, declared in the reviewed dataset
/// before any arm is scored.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    /// Fewer judged tasks than this makes every verdict `Inconclusive`.
    pub min_tasks: usize,
    /// The paired nDCG@k delta CI's lower bound must exceed this to improve.
    pub min_ndcg_gain: f64,
    /// The CI's upper bound must fall below its negation to regress.
    pub max_ndcg_regression: f64,
    /// Per-task p95 latency ceiling.
    pub max_p95_task_ms: u64,
}

/// A versioned, reviewed benchmark set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkSet {
    /// Schema version; must be [`SET_VERSION`].
    pub version: u32,
    /// Set name, carried into the artifact.
    pub name: String,
    /// Optional prose describing what the set covers.
    #[serde(default)]
    pub description: Option<String>,
    /// The pinned retrieval configuration. Required: a benchmark that inherits
    /// the live config is not reproducible.
    pub ranking: TuneCandidate,
    /// Present exactly when this is a BYO-vector set.
    #[serde(default)]
    pub vectors: Option<VectorSpec>,
    /// Set-wide defaults.
    #[serde(default)]
    pub defaults: Defaults,
    /// The budgets every arm's verdict is read against.
    pub budgets: Budgets,
    /// The reviewed tasks.
    pub tasks: Vec<BenchmarkTask>,
}

impl BenchmarkSet {
    /// Read and validate a set from a YAML file.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("benchmark set {}: {e}", path.display())))?;
        let set: BenchmarkSet = serde_yaml::from_str(&raw)
            .map_err(|e| Error::Config(format!("benchmark set {}: {e}", path.display())))?;
        set.validate()
            .map_err(|e| Error::Config(format!("benchmark set {}: {e}", path.display())))?;
        Ok(set)
    }

    /// The config this set scores with: the live one with its pinned `ranking`
    /// knobs swapped in and re-validated. A `BadRequest` from validation is
    /// converted, because the fault is in the file the operator wrote.
    pub fn effective_config(&self, base: &Config) -> Result<Config> {
        tune::validated_with_candidate(base, &self.ranking)
            .map_err(|e| Error::Config(format!("benchmark set `{}` ranking: {e}", self.name)))
    }

    /// Reject every shape that would score something other than what the set
    /// says. Messages carry the offending task id, never only a field name.
    fn validate(&self) -> Result<()> {
        if self.version != SET_VERSION {
            return Err(Error::Config(format!(
                "version {} is not supported (this binary reads version {SET_VERSION})",
                self.version
            )));
        }
        if self.name.trim().is_empty() {
            return Err(Error::Config("name must not be empty".into()));
        }
        if self.tasks.is_empty() {
            return Err(Error::Config("no tasks: nothing to score".into()));
        }
        self.validate_numbers()?;
        let mut seen: HashSet<&str> = HashSet::with_capacity(self.tasks.len());
        for task in &self.tasks {
            if !seen.insert(task.id.as_str()) {
                return Err(Error::Config(format!(
                    "duplicate task id `{}`: task ids key the report and every scores file",
                    task.id
                )));
            }
            self.validate_task(task)?;
        }
        Ok(())
    }

    /// Every numeric threshold the file declares, checked through one table so
    /// the budgets and the defaults cannot drift into two different rules.
    /// Budgets are required and finite because a verdict read against a
    /// placeholder is not a verdict.
    fn validate_numbers(&self) -> Result<()> {
        let b = &self.budgets;
        let checks: [(bool, &str); 6] = [
            (b.min_tasks >= 1, "budgets.min_tasks must be at least 1"),
            (
                b.min_ndcg_gain.is_finite() && b.min_ndcg_gain >= 0.0,
                "budgets.min_ndcg_gain must be finite and non-negative",
            ),
            (
                b.max_ndcg_regression.is_finite() && b.max_ndcg_regression >= 0.0,
                "budgets.max_ndcg_regression must be finite and non-negative",
            ),
            (
                b.max_p95_task_ms > 0,
                "budgets.max_p95_task_ms must be greater than 0",
            ),
            (self.defaults.k >= 1, "defaults.k must be at least 1"),
            (
                self.defaults.max_text_bytes >= 1,
                "defaults.max_text_bytes must be at least 1",
            ),
        ];
        match checks.iter().find(|(ok, _)| !ok) {
            Some((_, message)) => Err(Error::Config((*message).to_string())),
            None => Ok(()),
        }
    }

    /// One task: a real query, at least one judgment, no filter or judgment
    /// belonging to a domain the task is not scoped to, and a vector exactly
    /// when the set declares one.
    fn validate_task(&self, task: &BenchmarkTask) -> Result<()> {
        let id = &task.id;
        if id.trim().is_empty() {
            return Err(Error::Config("a task has an empty id".into()));
        }
        if task.query.trim().is_empty() {
            return Err(Error::Config(format!(
                "task `{id}`: query must not be empty"
            )));
        }
        if task.judgments.is_empty() {
            return Err(Error::Config(format!(
                "task `{id}`: no judgments, so the task carries no signal"
            )));
        }
        for judgment in &task.judgments {
            if judgment.relevance > MAX_RELEVANCE {
                return Err(Error::Config(format!(
                    "task `{id}`: relevance {} is above the {MAX_RELEVANCE} grade ceiling",
                    judgment.relevance
                )));
            }
        }
        let mask = task.domain.mask();
        for key in task.target_keys()? {
            if !mask.contains(domain_of(key.domain())) {
                return Err(Error::Config(format!(
                    "task `{id}`: a {} judgment cannot match under `domain: {}`",
                    key.domain().as_str(),
                    task.domain.as_str()
                )));
            }
        }
        validate_task_filters(id, &task.filters, task.domain.as_str(), mask)?;
        self.validate_task_vector(task)
    }

    /// A BYO-vector set requires every task to supply a vector of exactly
    /// `dim`; a lexical set requires none to supply one.
    fn validate_task_vector(&self, task: &BenchmarkTask) -> Result<()> {
        let id = &task.id;
        match (&self.vectors, &task.vector) {
            (None, None) => Ok(()),
            (Some(spec), Some(vector)) if vector.len() == spec.dim => Ok(()),
            (Some(spec), Some(vector)) => Err(Error::Config(format!(
                "task `{id}`: vector has {} values, but vectors.dim is {}",
                vector.len(),
                spec.dim
            ))),
            (Some(_), None) => Err(Error::Config(format!(
                "task `{id}`: this set declares `vectors`, so every task must supply one"
            ))),
            (None, Some(_)) => Err(Error::Config(format!(
                "task `{id}`: supplies a vector but the set declares no `vectors` model; \
                 lexical and BYO-vector are separate scenarios"
            ))),
        }
    }
}

#[cfg(test)]
#[path = "tests/benchmark_set.rs"]
mod tests;
