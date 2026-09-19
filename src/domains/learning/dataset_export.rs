//! `domains::learning::dataset_export::{Request, run}` — the middle of
//! `comemory export-dataset`: read the captured observations and the reviewed
//! verdicts, apply the export pipeline, and write a versioned JSONL dataset
//! plus its manifest (#210).
//!
//! Nothing here runs retrieval or writes to the database. Every number in a
//! record was measured at capture time and is read back verbatim, which is why
//! a record describes the corpus its observation saw rather than today's.
//!
//! The holdout split is withheld unless it is asked for by name, and every
//! file the command owns is cleared from the output directory before anything
//! is written — so a `holdout.jsonl` an earlier run left behind cannot survive
//! into a run that withholds it.

use std::path::PathBuf;

use serde::Deserialize;
use time::OffsetDateTime;

use crate::domains::learning::evaluation::candidate_identity::CandidateDomain;
use crate::domains::learning::evaluation::candidate_observation::OBSERVATION_VERSION;
use crate::domains::learning::evaluation::dataset_build::{self, BuildInput, Dataset};
use crate::domains::learning::evaluation::dataset_files::{write_buckets, write_manifest};
use crate::domains::learning::evaluation::dataset_manifest::{
    DatasetManifest, FileEntry, FilterReport, MANIFEST_VERSION, RowStats, SplitRatios, SplitReport,
    dataset_id, snapshot_digest,
};
use crate::domains::learning::evaluation::dataset_record::RECORD_VERSION;
use crate::domains::learning::evaluation::dataset_rows::{self, ProvenanceFilter, RowInput};
use crate::domains::learning::evaluation::dataset_split::{self, GroupInput, SplitConfig};
use crate::prelude::*;
use crate::store::candidate_dataset::snapshot;
use crate::store::{memory_row, migrate};
use crate::utilities::context::Ctx;
use crate::utilities::when::{DayEdge, parse_when};

/// Default salt for the group hash. Changing it reassigns every group, so it
/// is a declared constant rather than a value derived from the data.
pub const DEFAULT_SEED: &str = "comemory-dataset-v1";

/// Default train, validation and holdout ratios.
pub const DEFAULT_RATIOS: [f64; 3] = [0.7, 0.15, 0.15];

/// `comemory export-dataset` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Output directory, created if absent.
    pub out: PathBuf,
    /// `manual` (the default) | `implicit` | `all`.
    #[serde(default)]
    pub provenance: Option<String>,
    /// Emit a record for every retrieved candidate with no verdict.
    #[serde(default)]
    pub include_unjudged: bool,
    /// Write the holdout split's files.
    #[serde(default)]
    pub include_holdout: bool,
    /// Restrict records to these domains; empty means all three.
    #[serde(default)]
    pub domains: Vec<String>,
    /// Inclusive lower bound on the capture instant.
    #[serde(default)]
    pub since: Option<String>,
    /// Exclusive upper bound on the capture instant.
    #[serde(default)]
    pub until: Option<String>,
    /// `TRAIN,VALIDATION,HOLDOUT` ratios.
    #[serde(default)]
    pub split: Option<String>,
    /// Salt for the group hash.
    #[serde(default)]
    pub split_seed: Option<String>,
    /// Reserve every group holding a code candidate from this repo.
    #[serde(default)]
    pub holdout_repo: Option<String>,
    /// Reserve every group holding an observation captured at or after this.
    #[serde(default)]
    pub holdout_since: Option<String>,
    /// Reviewed-negative cap per query group per split; `0` is unlimited.
    #[serde(default)]
    pub max_negatives_per_query: usize,
}

/// What one export wrote.
pub struct ExportReport {
    /// The directory it wrote into.
    pub out: PathBuf,
    /// The manifest it wrote, as written.
    pub manifest: DatasetManifest,
}

/// Read the window, build the dataset, write the directory, return the report.
pub fn run(ctx: &mut Ctx<'_>, req: &Request) -> Result<ExportReport> {
    let settings = Settings::of(req)?;
    let conn = ctx.conn()?;
    let captured = snapshot(conn, settings.since.as_deref(), settings.until.as_deref())?;
    let digest = snapshot_digest(&captured);
    let built = dataset_rows::build(RowInput {
        snapshot: &captured,
        provenance: settings.provenance,
        domains: &settings.domains,
        include_unjudged: req.include_unjudged,
    })?;
    let plan = dataset_split::plan(&group_inputs(&built), &settings.split);

    let mut counts = built.stats.clone();
    let dataset = dataset_build::assemble(
        BuildInput {
            built: &built,
            plan: &plan,
            classes: &settings.provenance.classes(),
            max_negatives_per_query: req.max_negatives_per_query,
        },
        &mut counts,
    );
    let (files, withheld) = write_buckets(&req.out, &dataset, req.include_holdout)?;
    let manifest = manifest(ManifestParts {
        req,
        settings: &settings,
        split: split_report(&settings.split, &plan),
        digest,
        counts,
        dataset: &dataset,
        files,
        withheld,
    });
    write_manifest(&req.out, &manifest)?;
    Ok(ExportReport {
        out: req.out.clone(),
        manifest,
    })
}

/// Every request value validated before the database is opened, so a
/// malformed invocation creates no database and writes no file.
struct Settings {
    provenance: ProvenanceFilter,
    domains: Vec<CandidateDomain>,
    since: Option<String>,
    until: Option<String>,
    split: SplitConfig,
}

impl Settings {
    /// Validate one request.
    fn of(req: &Request) -> Result<Settings> {
        Ok(Settings {
            provenance: ProvenanceFilter::parse(req.provenance.as_deref().unwrap_or("manual"))?,
            domains: domains(&req.domains)?,
            since: instant(req.since.as_deref(), DayEdge::Start)?,
            until: instant(req.until.as_deref(), DayEdge::End)?,
            split: SplitConfig {
                seed: req
                    .split_seed
                    .clone()
                    .unwrap_or_else(|| DEFAULT_SEED.into()),
                ratios: ratios(req.split.as_deref())?,
                holdout_repo: req.holdout_repo.clone(),
                holdout_since: instant(req.holdout_since.as_deref(), DayEdge::Start)?,
            },
        })
    }
}

/// The per-row graph inputs, read off the built rows and their contexts.
fn group_inputs(built: &dataset_rows::BuiltRows) -> Vec<GroupInput<'_>> {
    built
        .rows
        .iter()
        .map(|row| {
            let ctx = built.contexts.get(row.observation);
            GroupInput {
                query_group: ctx.map_or("", |c| c.query_group.as_str()),
                content_group: &row.content_group,
                repo: row.repo.as_deref(),
                at: ctx.map_or("", |c| c.at.as_str()),
            }
        })
        .collect()
}

/// The realized split report.
fn split_report(cfg: &SplitConfig, plan: &dataset_split::SplitPlan) -> SplitReport {
    SplitReport {
        policy: "grouped-hash".to_string(),
        seed: cfg.seed.clone(),
        ratios: SplitRatios {
            train: cfg.ratios[0],
            validation: cfg.ratios[1],
            holdout: cfg.ratios[2],
        },
        holdout_repo: cfg.holdout_repo.clone(),
        holdout_since: cfg.holdout_since.clone(),
        groups: plan.assignments.len(),
        largest_group_rows: plan.assignments.iter().map(|a| a.rows).max().unwrap_or(0),
        assignments: plan.assignments.clone(),
    }
}

/// Everything the manifest assembly needs, bundled rather than positional.
struct ManifestParts<'a> {
    req: &'a Request,
    settings: &'a Settings,
    split: SplitReport,
    digest: String,
    counts: RowStats,
    dataset: &'a Dataset,
    files: Vec<FileEntry>,
    withheld: Vec<FileEntry>,
}

/// The manifest, with its derived `dataset_id`.
fn manifest(parts: ManifestParts<'_>) -> DatasetManifest {
    let filters = FilterReport {
        provenance: parts.settings.provenance.as_str().to_string(),
        include_unjudged: parts.req.include_unjudged,
        include_holdout: parts.req.include_holdout,
        domains: parts
            .settings
            .domains
            .iter()
            .map(|d| d.as_str().to_string())
            .collect(),
        since: parts.settings.since.clone(),
        until: parts.settings.until.clone(),
        max_negatives_per_query: parts.req.max_negatives_per_query,
    };
    DatasetManifest {
        manifest_version: MANIFEST_VERSION,
        record_version: RECORD_VERSION,
        observation_version: OBSERVATION_VERSION,
        dataset_id: dataset_id(&filters, &parts.split, &parts.digest),
        snapshot_digest: parts.digest,
        binary_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version: migrate::CURRENT_VERSION.to_string(),
        filters,
        split: parts.split,
        counts: parts.counts,
        by_domain: parts.dataset.by_domain.clone(),
        by_split: parts.dataset.by_split.clone(),
        by_label: parts.dataset.by_label.clone(),
        retrieval_revisions: parts.dataset.revisions.clone(),
        files: parts.files,
        withheld: parts.withheld,
    }
}

/// The domains in scope, ascending and deduplicated. An empty request means
/// all three; an unknown word is refused naming what is accepted.
///
/// Deliberately not `CandidateDomain::from_label`, which is total and maps any
/// unrecognised label to `Memory` — correct where it is used, since a stored
/// `UnifiedHit::domain` is one of three known literals, and wrong here, where
/// `--domain memories` must be a refusal rather than a silent memory-only
/// export.
fn domains(requested: &[String]) -> Result<Vec<CandidateDomain>> {
    if requested.is_empty() {
        return Ok(CandidateDomain::all().to_vec());
    }
    let mut chosen = Vec::with_capacity(requested.len());
    for word in requested {
        let found = CandidateDomain::all()
            .into_iter()
            .find(|d| d.as_str() == word)
            .ok_or_else(|| {
                Error::Config(format!(
                    "unknown domain `{word}`: expected one of memory, code, document"
                ))
            })?;
        if !chosen.contains(&found) {
            chosen.push(found);
        }
    }
    chosen.sort_unstable();
    Ok(chosen)
}

/// The three ratios, validated before the database opens.
fn ratios(raw: Option<&str>) -> Result<[f64; 3]> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_RATIOS);
    };
    let parsed: Vec<f64> = raw
        .split(',')
        .map(|part| {
            part.trim().parse::<f64>().map_err(|_| {
                Error::Usage(format!(
                    "invalid --split `{raw}`: expected TRAIN,VALIDATION,HOLDOUT as three numbers"
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let [train, validation, holdout] = parsed.as_slice() else {
        return Err(Error::Usage(format!(
            "invalid --split `{raw}`: expected exactly three comma-separated ratios"
        )));
    };
    let all = [*train, *validation, *holdout];
    if all.iter().any(|r| !r.is_finite() || *r < 0.0) {
        return Err(Error::Usage(format!(
            "invalid --split `{raw}`: every ratio must be finite and at least 0"
        )));
    }
    if (all.iter().sum::<f64>() - 1.0).abs() > 1e-9 {
        return Err(Error::Usage(format!(
            "invalid --split `{raw}`: the three ratios must sum to 1.0"
        )));
    }
    Ok(all)
}

/// One optional temporal bound, rendered in the fixed-width shape the `at`
/// column is written in so a plain string comparison is chronological.
fn instant(raw: Option<&str>, edge: DayEdge) -> Result<Option<String>> {
    match raw {
        None => Ok(None),
        Some(value) => {
            let parsed: OffsetDateTime = parse_when(value, edge)?;
            memory_row::iso_format(parsed).map(Some)
        }
    }
}

#[cfg(test)]
#[path = "tests/dataset_export.rs"]
mod tests;
