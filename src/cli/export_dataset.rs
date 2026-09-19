//! `comemory export-dataset` — write a versioned JSONL relevance dataset and
//! its manifest from the candidate observations `comemory find` captured and
//! the verdicts `comemory judge` recorded.
//!
//! CLI-only, like `benchmark`: the run writes a directory of files to an
//! operator-named filesystem path, which a server must never do on request.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::load_config;
use crate::cli::output::json;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::learning::dataset_export::{self, ExportReport};
use crate::domains::learning::evaluation::candidate_identity::CandidateDomain;
use crate::domains::learning::evaluation::dataset_manifest::FileEntry;
use crate::domains::learning::evaluation::dataset_rows::ProvenanceFilter;
use crate::prelude::*;
use crate::store::connection;
use crate::utilities::context::Ctx;

const EXAMPLES: &str = "\
Examples:
  # Reviewed verdicts only, holdout withheld (the defaults)
  comemory export-dataset --out ./dataset

  # Include the pool a reviewer did not judge, marked label: null
  comemory export-dataset --out ./dataset --include-unjudged

  # Reserve one repository as the qualification split, then release it
  comemory export-dataset --out ./dataset --holdout-repo comemory
  comemory export-dataset --out ./qualify --holdout-repo comemory --include-holdout";

/// Arguments to `comemory export-dataset`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Output directory (created if absent). Every file this command owns is
    /// removed from it before writing; nothing else is touched.
    #[arg(long)]
    pub out: PathBuf,
    /// Which judgment provenance reaches the dataset. Implicit labels are
    /// never written to the reviewed files.
    ///
    /// The accepted words are read off `ProvenanceFilter::WORDS`, so clap can
    /// refuse a typo before the database opens without the vocabulary being
    /// spelled a second time. The core still validates: `Request` derives
    /// `Deserialize`, so it must refuse a bad word whatever the caller is.
    #[arg(
        long,
        value_name = "WORD",
        default_value = "manual",
        value_parser = clap::builder::PossibleValuesParser::new(ProvenanceFilter::WORDS)
    )]
    pub provenance: String,
    /// Also emit a record for every retrieved candidate nobody judged,
    /// carrying `label: null`. Never a relevance of 0.
    #[arg(long)]
    pub include_unjudged: bool,
    /// Also write the holdout split. Withheld by default, so training-time
    /// mining and model selection have no qualification file to read.
    #[arg(long)]
    pub include_holdout: bool,
    /// Restrict records to a domain (`memory`, `code`, `document`).
    /// Repeatable; all three by default.
    ///
    /// The accepted words are the contract's own `CandidateDomain` spellings,
    /// read off the enum rather than restated here.
    #[arg(
        long = "domain",
        value_name = "DOMAIN",
        value_parser = clap::builder::PossibleValuesParser::new(
            CandidateDomain::all().map(CandidateDomain::as_str)
        )
    )]
    pub domains: Vec<String>,
    /// Only observations captured at or after this instant.
    #[arg(long, value_name = "WHEN")]
    pub since: Option<String>,
    /// Only observations captured before this instant.
    #[arg(long, value_name = "WHEN")]
    pub until: Option<String>,
    /// Train, validation and holdout ratios, summing to 1.0.
    #[arg(long, value_name = "TRAIN,VALIDATION,HOLDOUT")]
    pub split: Option<String>,
    /// Salt for the group hash. Changing it reassigns every group.
    #[arg(long, value_name = "STRING")]
    pub split_seed: Option<String>,
    /// Reserve every group holding a code candidate from this repo for the
    /// holdout split, whatever its hash says.
    #[arg(long, value_name = "LABEL")]
    pub holdout_repo: Option<String>,
    /// Reserve every group holding an observation captured at or after this
    /// instant for the holdout split.
    #[arg(long, value_name = "WHEN")]
    pub holdout_since: Option<String>,
    /// Cap how many reviewed relevance-0 records one query group contributes
    /// to one split. 0 is unlimited.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub max_negatives_per_query: usize,
}

/// Run `comemory export-dataset`: build the dataset, write the directory, and
/// report what was written and what was refused.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let report = dataset_export::run(
        &mut ctx,
        &dataset_export::Request {
            out: a.out,
            provenance: Some(a.provenance),
            include_unjudged: a.include_unjudged,
            include_holdout: a.include_holdout,
            domains: a.domains,
            since: a.since,
            until: a.until,
            split: a.split,
            split_seed: a.split_seed,
            holdout_repo: a.holdout_repo,
            holdout_since: a.holdout_since,
            max_negatives_per_query: a.max_negatives_per_query,
        },
    )?;
    if json_flag {
        return json::write(&report.manifest);
    }
    write_tty(&report)
}

/// The human summary: what was written, what was withheld, and every number
/// the export refused to guess at.
fn write_tty(report: &ExportReport) -> Result<()> {
    let manifest = &report.manifest;
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} -> {}  ({} record(s), {} group(s), record v{})",
        manifest.dataset_id,
        report.out.display(),
        manifest.counts.rows_emitted,
        manifest.split.groups,
        manifest.record_version
    )?;
    for entry in &manifest.files {
        writeln!(out, "  {}", line_of(entry))?;
    }
    for entry in &manifest.withheld {
        writeln!(out, "  {}  WITHHELD", line_of(entry))?;
    }
    let counts = &manifest.counts;
    writeln!(
        out,
        "unjudged {}  unresolved {} (judged {})  stale {}  pool miss {}  \
         contradictions {}  duplicates {}  refused version {}",
        counts.candidates_unjudged,
        counts.candidates_unresolved,
        counts.judgments_on_unresolved_candidates,
        counts.judgments_stale,
        counts.judgments_recall_miss,
        counts.judgments_contradictory_dropped,
        counts.duplicate_observations_dropped,
        counts.observations_refused_version
    )?;
    if counts.observations_scanned == 0 {
        writeln!(
            out,
            "no candidate observations were captured in this window; \
             set COMEMORY_OBSERVATIONS_ENABLED=1 and run comemory find to record one"
        )?;
    }
    Ok(())
}

/// One file line: its name, its rows, and the leading characters of its digest
/// — enough to compare two exports by eye.
///
/// `sha256` is always 64 characters here, so the fallback is unreachable; it
/// exists because slicing a string is fallible and this report line must never
/// be the thing that fails an export.
fn line_of(entry: &FileEntry) -> String {
    let digest = entry.sha256.get(..12).unwrap_or(entry.sha256.as_str());
    format!(
        "{:<28} {:>6} row(s)  {:>9} byte(s)  {digest}",
        entry.path, entry.rows, entry.bytes
    )
}
