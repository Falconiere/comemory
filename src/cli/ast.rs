//! `comemory ast` — run an ast-grep pattern against a single source file and
//! print every match's `(file:line  text)` row. Language is required so we
//! pick the right tree-sitter grammar without sniffing extensions, and is
//! gated against the compiled-in language set so callers get a clear error
//! for unsupported values instead of a silent grammar mismatch.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::ast::languages::{self, Lang};
use crate::ast::pattern::find;
use crate::cli::pagination::PaginationArgs;
use crate::output::page::Page;
use crate::output::{json, tty};
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Match every fn returning Result<_>
  comemory ast 'fn $NAME($$$ARGS) -> Result<$RET>' --lang rs --file src/db.rs

  # Find tokio::spawn call sites
  comemory ast 'tokio::spawn($$$)' --lang rs --file src/lib.rs --json

  # Hunt for `console.log` left in TypeScript
  comemory ast 'console.log($$$)' --lang ts --file src/index.ts";

/// Arguments to `comemory ast`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// ast-grep pattern (`$VAR`, `$$$ARGS`, etc.).
    pub pattern: String,
    /// Language tag: `rs`/`rust`, `ts`/`tsx`/`typescript`, `js`/`jsx`/`javascript`,
    /// `py`/`python`, `go`.
    #[arg(long)]
    pub lang: String,
    /// File to search.
    #[arg(long)]
    pub file: PathBuf,
    /// `--limit` / `--offset` window over the matches.
    #[command(flatten)]
    pub page: PaginationArgs,
}

/// One row of `comemory ast` output (mirrors the `(line, text)` shape returned
/// by `ast::pattern::find`).
#[derive(Serialize)]
struct Row {
    line: usize,
    text: String,
}

/// Read the file, run the pattern, and print matches.
pub async fn run(a: Args, json_flag: bool, _data_dir: Option<PathBuf>) -> Result<()> {
    let lang = Lang::parse(&a.lang).ok_or_else(|| {
        Error::Config(format!(
            "unsupported --lang {:?}; supported: {}",
            a.lang,
            languages::supported().join(", ")
        ))
    })?;
    let src = std::fs::read_to_string(&a.file)?;
    let hits = find(lang, &src, &a.pattern)?;
    let rows: Vec<Row> = hits
        .into_iter()
        .map(|(line, text)| Row { line, text })
        .collect();
    let page = Page::from_slice(rows, a.page.limit, a.page.offset);

    if json_flag {
        json::write(&page)?;
    } else {
        let mut out = std::io::stdout().lock();
        for r in &page.items {
            writeln!(out, "{}:{}  {}", a.file.display(), r.line, r.text)?;
        }
        tty::write_page_footer(&mut out, page.items.len(), page.offset, page.total)?;
    }
    Ok(())
}
