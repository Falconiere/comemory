//! `comemory ast` — run an ast-grep pattern against a single source file and
//! print every match's `(file:line  text)` row. Language is required so we
//! pick the right tree-sitter grammar without sniffing extensions, and is
//! gated against the compiled-in language set so callers get a clear error
//! for unsupported values instead of a silent grammar mismatch.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::pagination::PaginationArgs;
use crate::config::Config;
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

/// Read the file, run the pattern, and print matches. `ast` has no
/// `Paths`/db dependency, so `data_dir` resolves a throwaway `Ctx::lazy` that
/// is never opened.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let cfg = Config::defaults();
    let paths = crate::config::Paths::new(crate::config::paths::resolve_data_dir(data_dir));
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let req = api::ast::Request {
        pattern: a.pattern,
        lang: a.lang,
        file: a.file.display().to_string(),
        limit: a.page.limit,
        offset: a.page.offset,
    };
    let page = api::ast::run(&mut ctx, req)?;

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
