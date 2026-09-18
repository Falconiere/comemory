//! `comemory judge` — record reviewed relevance verdicts against a candidate
//! observation `comemory find` captured, or report that observation.
//!
//! CLI-only, like `benchmark`: a verdict against a locally captured
//! observation is a local review action, and the two HTTP feedback routes keep
//! their own unchanged contract. There is no `--source` flag for the reason
//! `comemory feedback` has none (#130) — a typed verdict is a human one, so it
//! is always recorded as `manual`.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::load_config;
use crate::cli::output::json;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::learning::judge;
use crate::prelude::*;
use crate::utilities::context::Ctx;

const EXAMPLES: &str = "\
Examples:
  # Capture a pool, then look at what it recorded
  COMEMORY_OBSERVATIONS_ENABLED=1 comemory find \"frontmatter contract\"
  comemory judge o-20260918-9f8e7d6c

  # Grade three candidates, one per domain (0 = reviewed and not relevant)
  comemory judge o-20260918-9f8e7d6c \\
    --ref 'memory:5a9f19bc:5a9f19bc403e...=3' \\
    --ref 'code:comemory:src/domains/retrieval/rerank.rs:rerank:9d1c1f0b=2' \\
    --ref 'document:0f1e2d3c:guides/schema-migrations.md:7b2a9c:3=0'";

/// Arguments to `comemory judge`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Id of the captured observation (`o-<yyyymmdd>-<8hex>`, as printed by
    /// `comemory find` when candidate capture is enabled).
    pub observation_id: String,
    /// A verdict, as `<candidate_ref>=<relevance>` with relevance in 0..=3.
    /// Repeatable. With none, the observation is reported and nothing is
    /// written.
    #[arg(long = "ref", value_name = "REF=RELEVANCE")]
    pub refs: Vec<String>,
}

/// Run `comemory judge`: resolve every verdict against the observation and
/// record them all, or report the observation when no verdict was given.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let outcome = judge::run(
        &mut ctx,
        judge::Request {
            observation: a.observation_id,
            refs: a.refs,
            // A typed verdict is a human one: `manual`, always. Only an
            // implicit writer would set this, and none exists on the CLI.
            source: None,
        },
    )?;
    match outcome {
        judge::Outcome::Recorded(r) => emit_recorded(json_flag, &r),
        judge::Outcome::Report(r) => emit_report(json_flag, &r),
    }
}

/// The acknowledgement of a recording run.
fn emit_recorded(json_flag: bool, recorded: &judge::Recorded) -> Result<()> {
    if json_flag {
        return json::write(recorded);
    }
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "recorded {} judgment(s) as {} against {}",
        recorded.recorded, recorded.provenance, recorded.observation_id
    )?;
    Ok(())
}

/// The read-only view of one observation.
fn emit_report(json_flag: bool, report: &judge::Report) -> Result<()> {
    if json_flag {
        return json::write(report);
    }
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{}  v{}  \"{}\"",
        report.observation_id, report.observation_version, report.query
    )?;
    writeln!(
        out,
        "corpus {}  knobs {}  {} candidate(s){}",
        short(&report.corpus_digest),
        short(&report.knobs_hash),
        report.candidate_count,
        if report.truncated {
            " (POOL TRUNCATED: pool recall is not measurable from this observation)"
        } else {
            ""
        }
    )?;
    for c in &report.candidates {
        writeln!(
            out,
            "{:>3}{:>5}  {:<8} {:<2} {}",
            c.pool_position,
            c.returned_position
                .map_or_else(|| "-".to_string(), |p| p.to_string()),
            c.domain,
            c.relevance
                .map_or_else(|| ".".to_string(), |r| r.to_string()),
            c.title
        )?;
        writeln!(
            out,
            "          {}{}",
            c.candidate_ref,
            if c.unresolved { "  [unresolved]" } else { "" }
        )?;
    }
    Ok(())
}

/// The first 12 characters of a digest, enough to compare two runs by eye.
fn short(digest: &str) -> &str {
    digest.get(..12).unwrap_or(digest)
}
