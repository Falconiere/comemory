//! `comemory distill` — extract explicit saves from a transcript and propose candidates.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::capture::distill::{DistillRequest, run as distill_run};
use crate::cli::off_runtime::off_runtime;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;
use crate::sync::auth_file::AuthFile;

const EXAMPLES: &str = "\
Examples:
  comemory distill --session-id <uuid> --transcript ~/.claude/projects/.../session.jsonl --dry-run
  comemory distill --session-id <uuid> --transcript ./session.jsonl --json
  comemory distill --session-id <uuid> --transcript ./session.jsonl --api-url https://dev-api.comemory.io";

/// Arguments to `comemory distill`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Platform session id from a prior `POST /v1/sessions` capture receipt.
    #[arg(long, value_name = "ID")]
    pub session_id: String,
    /// Path to a Claude Code JSONL transcript.
    #[arg(long, value_name = "PATH")]
    pub transcript: PathBuf,
    /// Build and print the candidate batch without POSTing.
    #[arg(long)]
    pub dry_run: bool,
    /// Override the platform API base (defaults to the URL stored in auth.json).
    #[arg(long, value_name = "URL")]
    pub api_url: Option<String>,
}

/// Extract explicit-save candidates and optionally POST them.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let req = DistillRequest {
        session_id: a.session_id,
        transcript: a.transcript,
        dry_run: a.dry_run,
        api_url: a.api_url,
    };

    let report = if a.dry_run {
        distill_run(None, &req)?
    } else {
        let auth = AuthFile::load(&paths)?
            .ok_or_else(|| Error::Usage("not logged in — run `comemory auth login`".into()))?;
        off_runtime(move || distill_run(Some(&auth), &req))?
    };

    if json_flag {
        return json::write(&report);
    }

    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "distill session={} extracted={} dry_run={}",
        report.session_id, report.extracted, report.dry_run
    )?;
    for (i, c) in report.batch.candidates.iter().enumerate() {
        writeln!(
            out,
            "  [{i}] {} {:?} tags={:?} confidence={}",
            c.kind,
            c.title,
            c.tags.clone().unwrap_or_default(),
            c.confidence
        )?;
    }
    if let Some(resp) = &report.response {
        writeln!(
            out,
            "posted stored={} duplicates={}",
            resp.stored, resp.duplicates
        )?;
        for (i, r) in resp.results.iter().enumerate() {
            writeln!(
                out,
                "  result[{i}] status={} state={:?}",
                r.status,
                r.state.clone().unwrap_or_default()
            )?;
        }
    }
    Ok(())
}
