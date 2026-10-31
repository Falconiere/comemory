//! Selected-folder orchestration for `comemory run embedding` and HTTP jobs.

use serde::{Deserialize, Serialize};

use crate::domains::code::index_code::{self, IndexMode};
use crate::domains::documents::{index, source::SourceId};
use crate::prelude::*;
use crate::utilities::{context::Ctx, progress::ProgressSink};

/// One explicit folder reconciliation; path selection never follows Git metadata.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Selected directory (defaults to the invocation's current directory).
    #[serde(default = "default_path")]
    pub path: String,
    /// Optional document repository metadata, independent of the root.
    #[serde(default)]
    pub repo: Option<String>,
    /// Incremental preserves eligible unchanged derived data; full refreshes it.
    #[serde(default)]
    pub mode: IndexMode,
}

fn default_path() -> String {
    ".".into()
}

/// Per-domain outcomes for the selected folder.
#[derive(Debug, Serialize)]
pub struct Response {
    /// Document registration, extraction and embedding outcomes.
    pub documents: index::Output,
    /// Code extraction and embedding outcomes, isolated by canonical root.
    pub code: index_code::Response,
    /// Configuration diagnostics; lexical indexing remains available.
    pub warnings: Vec<String>,
}

/// Reconcile documents and code through their existing shared cores.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    run_with_progress(ctx, req, None)
}

/// Reconcile with cooperative cancellation between and within domain passes.
pub fn run_with_progress(
    ctx: &mut Ctx<'_>,
    req: Request,
    sink: Option<&dyn ProgressSink>,
) -> Result<Response> {
    if sink.is_some_and(ProgressSink::is_cancelled) {
        return Err(Error::Cancelled);
    }
    let root = std::fs::canonicalize(&req.path)
        .map_err(|e| Error::Usage(format!("embedding: cannot resolve selected folder: {e}")))?;
    if !root.is_dir() {
        return Err(Error::Usage(
            "embedding: --path must name a directory".into(),
        ));
    }
    if let Some(label) = req
        .repo
        .clone()
        .or_else(|| crate::domains::code::git_utils::repo_label_at(&root))
    {
        index_code::refuse_if_archived(ctx.conn()?, &label)?;
    }
    let path = root.to_string_lossy().into_owned();
    let documents = index::run_with_mode(
        ctx,
        index::Request {
            path: vec![path.clone()],
            repo: req.repo,
            strict: false,
        },
        sink,
        req.mode == IndexMode::Full,
    )?;
    let code = index_code::run_folder_with_progress(
        ctx,
        index_code::Request {
            repo: format!("source-{}", SourceId::from_canonical_path(&root)),
            path,
            mode: req.mode,
        },
        sink,
    )?;
    crate::store::code_labels::set(
        ctx.conn()?,
        &code.repo,
        documents
            .sources
            .first()
            .and_then(|source| source.repo.as_deref()),
    )?;
    let warnings = if ctx.cfg.embed_cmd.is_some() && ctx.cfg.code_embed_cmd.is_none() {
        vec!["Code semantic indexing needs a compatible 768-dimensional provider; configure COMEMORY_CODE_EMBED_CMD and rerun. Memory embeddings cannot be reused for code.".into()]
    } else {
        Vec::new()
    };
    Ok(Response {
        documents,
        code,
        warnings,
    })
}
