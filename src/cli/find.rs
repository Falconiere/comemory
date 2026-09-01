//! `comemory find` — one ranked list across memories, code, and documents.
//!
//! The fusion lives in `retrieval::unified` and the shared middle in
//! `api::find` (Binding Rule 1). `search` and `search-code` are unchanged
//! and remain the right call when you want one domain's own hit shape.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::{embedding_input, load_config, track_searches};
use crate::config::paths::{Paths, resolve_data_dir};
use crate::memory::Kind;
use crate::output::json;
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # One ranked list across every domain
  comemory find \"frontmatter contract\"

  # Just the code domain — same ordering as `comemory search-code`
  comemory find \"parse_frontmatter\" --domain code

  # JSON; every hit carries `domain`, `rank_in_domain`, and that domain's
  # own `score_parts` object verbatim
  comemory find \"rrf fusion\" --json

  # The document leg's weight relative to memory and code (both 1.0) is
  # COMEMORY_RETRIEVAL_DOCUMENT_LEG_WEIGHT (default 0.5)
  COMEMORY_RETRIEVAL_DOCUMENT_LEG_WEIGHT=1.5 comemory find \"upgrade guide\"";

/// Arguments to `comemory find`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Natural-language query string.
    pub query: String,
    /// Page size — overrides the configured `retrieval.top_k`.
    #[arg(long, visible_alias = "limit")]
    pub k: Option<usize>,
    /// Ranked results to skip (deep paging).
    #[arg(long, default_value_t = 0)]
    pub offset: usize,
    /// Restrict to one domain.
    #[arg(long, default_value = "all")]
    pub domain: String,
    /// Repo filter. Narrows the memory and code legs.
    #[arg(long)]
    pub repo: Option<String>,
    /// Memory-kind filter. Narrows the memory leg only.
    #[arg(long)]
    pub kind: Option<Kind>,
    /// Language filter. Narrows the code leg only.
    #[arg(long)]
    pub lang: Option<String>,
    /// Document path glob (repeatable). Narrows the document leg only.
    #[arg(long = "path", value_name = "GLOB")]
    pub path: Vec<String>,
    /// Caller-supplied dense vector as a comma-separated float list.
    #[arg(long)]
    pub vector: Option<String>,
    /// Read a JSON `{ "embedding": [..] }` payload from stdin.
    #[arg(long, default_value_t = false)]
    pub vector_stdin: bool,
    /// Only consider memories created at or after this instant.
    #[arg(long, value_name = "WHEN")]
    pub since: Option<String>,
    /// Only consider memories created at or before this instant.
    #[arg(long, value_name = "WHEN")]
    pub until: Option<String>,
    /// Search the corpus as it stood at this instant.
    #[arg(long = "as-of", value_name = "WHEN", conflicts_with = "until")]
    pub as_of: Option<String>,
}

/// Run `comemory find`.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;
    let vector = embedding_input::read_optional(a.vector_stdin, a.vector.as_deref())?;

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let result = api::find::run(
        &mut ctx,
        api::find::Request {
            query: a.query,
            k: a.k,
            offset: a.offset,
            domain: Some(a.domain),
            repo: a.repo,
            kind: a.kind,
            lang: a.lang,
            path: a.path,
            vector,
            since: a.since,
            until: a.until,
            as_of: a.as_of,
        },
        track_searches()?,
    )?;

    if json_flag {
        json::write(&serde_json::json!({
            "hits": result.hits,
            "query_id": result.query_id,
            "limit": result.meta.limit,
            "offset": result.meta.offset,
            "has_more": result.meta.has_more,
            "total": result.meta.total,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        for hit in &result.hits {
            writeln!(out, "{:.4}  {:<8}  {}", hit.score, hit.domain, hit.title)?;
            writeln!(out, "          {}", hit.subtitle)?;
        }
    }
    Ok(())
}
