//! `api::ast::{Request, run}` — the shared middle of `comemory ast` / `POST
//! /api/v1/code/ast`: run an ast-grep pattern against one source file and
//! page the `(line, text)` matches. Moved out of `cli::ast::run` (Binding
//! Rule 1).
//!
//! Conn-free — `run` never calls [`Ctx::conn`].
//!
//! **Containment is not this file's job.** The HTTP route handler
//! canonicalizes and contains `req.file` (`security::contain_abs`) before
//! calling [`run`]; this middle stays exactly as unrestricted as the CLI.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::ast::languages::{self, Lang};
use crate::ast::pattern::find;
use crate::output::page::Page;
use crate::prelude::*;

/// `comemory ast` / `POST /api/v1/code/ast` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// ast-grep pattern (`$VAR`, `$$$ARGS`, etc.).
    pub pattern: String,
    /// Language tag: `rs`/`rust`, `ts`/`tsx`/`typescript`, `js`/`jsx`/`javascript`,
    /// `py`/`python`, `go`.
    pub lang: String,
    /// File to search.
    pub file: String,
    /// `--limit` window over the matches. `0` means "all". Defaults to the
    /// CLI's `PaginationArgs` default (50).
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// `--offset` — number of leading matches to skip.
    #[serde(default)]
    pub offset: usize,
}

/// `PaginationArgs`' CLI default (`--limit`, unset = 50), reused here so an
/// HTTP request omitting `limit` pages identically to the CLI.
fn default_limit() -> usize {
    50
}

/// One row of `comemory ast` output (mirrors the `(line, text)` shape
/// returned by `ast::pattern::find`).
#[derive(Serialize, Debug)]
pub struct Row {
    /// 1-based line number of the match.
    pub line: usize,
    /// Matched text.
    pub text: String,
}

/// Read `req.file`, run `req.pattern` against it under `req.lang`, and page
/// the matches.
pub fn run(_ctx: &mut Ctx<'_>, req: Request) -> Result<Page<Row>> {
    let lang = Lang::parse(&req.lang).ok_or_else(|| {
        Error::Config(format!(
            "unsupported --lang {:?}; supported: {}",
            req.lang,
            languages::supported().join(", ")
        ))
    })?;
    let src = std::fs::read_to_string(PathBuf::from(&req.file))?;
    let hits = find(lang, &src, &req.pattern)?;
    let rows: Vec<Row> = hits
        .into_iter()
        .map(|(line, text)| Row { line, text })
        .collect();
    Ok(Page::from_slice(rows, req.limit, req.offset))
}
