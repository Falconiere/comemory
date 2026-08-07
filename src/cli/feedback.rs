//! `comemory feedback` — record per-memory used/irrelevant feedback into the
//! SQLite stats database. Accepts comma-separated id lists for each side.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api;
use crate::cli::csv_unique;
use crate::config::Config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Mark two hits as useful and one as irrelevant
  comemory feedback q-20260610-a1b2c3d4 --used a1b2c3d4,e5f6a7b8 --irrelevant 00112233

  # Only-used feedback
  comemory feedback q-20260610-b2c3d4e5 --used a1b2c3d4

  # Only-irrelevant feedback
  comemory feedback q-20260610-c3d4e5f6 --irrelevant 00112233

  # Code-symbol feedback (ids printed by comemory search-code)
  comemory feedback q-20260610-d4e5f6a7 --used-code 12 --irrelevant-code 13

  # Memory and code verdicts in one call
  comemory feedback q-20260610-e5f6a7b8 --used a1b2c3d4 --used-code 12";

/// Arguments to `comemory feedback`. `query_id` is the
/// `q-<yyyymmdd>-<8hex>` id printed by `comemory search`; each verdict is
/// recorded against it in `feedback_events`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Id of the originating search query (`q-<yyyymmdd>-<8hex>`, as
    /// printed by `comemory search`); recorded for provenance.
    pub query_id: String,
    /// Comma-separated memory ids that were used.
    #[arg(long, default_value = "")]
    pub used: String,
    /// Comma-separated memory ids that were judged irrelevant.
    #[arg(long, default_value = "")]
    pub irrelevant: String,
    /// Comma-separated code-symbol ids (positive integers, as printed by
    /// `comemory search-code`) that were used.
    #[arg(long, default_value = "")]
    pub used_code: String,
    /// Comma-separated code-symbol ids that were judged irrelevant.
    #[arg(long, default_value = "")]
    pub irrelevant_code: String,
}

/// Parse `Args` into an [`api::feedback::Request`], run the shared middle
/// (Binding Rule 1 — shared with `POST /api/v1/feedback`), and emit a
/// one-line ack (or a JSON envelope with the recorded counts under
/// `--json`).
///
/// Uses a lazy `Ctx` (no `config.toml` read, no data-dir/DB touch) so
/// `api::feedback::run`'s validation runs first, exactly as
/// `cli::feedback::run` did pre-extraction (AC-13: a malformed query id or
/// id list must not create the data dir or `comemory.db`).
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = Config::defaults();
    let mut ctx = api::Ctx::lazy(&paths, &cfg);

    let req = api::feedback::Request {
        query_id: a.query_id,
        used: csv_unique(&a.used),
        irrelevant: csv_unique(&a.irrelevant),
        used_code: csv_unique(&a.used_code),
        irrelevant_code: csv_unique(&a.irrelevant_code),
    };
    let output = api::feedback::run(&mut ctx, req)?;
    emit(json_flag, &output)
}

/// Emit the feedback ack: a JSON object under `--json` (byte-identical to
/// the pre-extraction shape, including the ad-hoc `"ok"` field the
/// `/api/v1` envelope's own `ok` makes redundant over HTTP), else a TTY
/// line that surfaces the orphan-query-id notice (invisible at the default
/// `tracing` `EnvFilter` level).
fn emit(json_flag: bool, output: &api::feedback::Response) -> Result<()> {
    if json_flag {
        json::write(&serde_json::json!({
            "ok": true,
            "used": output.used,
            "irrelevant": output.irrelevant,
            "used_code": output.used_code,
            "irrelevant_code": output.irrelevant_code,
            "query_id": output.query_id,
            "known_query": output.known_query,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        if output.known_query {
            writeln!(out, "ok")?;
        } else {
            writeln!(
                out,
                "ok (query id not in log — evicted or never logged; recorded anyway)"
            )?;
        }
    }
    Ok(())
}
