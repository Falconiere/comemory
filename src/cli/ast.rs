//! `qwick ast` — run an ast-grep pattern against a single source file and
//! print every match's `(file:line  text)` row. Language is required so we
//! pick the right tree-sitter grammar without sniffing extensions.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::ast::pattern::find;
use crate::ast::Lang;
use crate::output::json;
use crate::prelude::*;

/// Arguments to `qwick ast`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// ast-grep pattern (`$VAR`, `$$$ARGS`, etc.).
    pub pattern: String,
    /// Language tag: `rs`/`rust`, `ts`/`tsx`, `js`/`jsx`, `py`.
    #[arg(long)]
    pub lang: String,
    /// File to search.
    #[arg(long)]
    pub file: PathBuf,
}

/// One row of `qwick ast` output (mirrors the `(line, text)` shape returned
/// by `ast::pattern::find`).
#[derive(Serialize)]
struct Row {
    line: usize,
    text: String,
}

/// Read the file, run the pattern, and print matches.
pub async fn run(a: Args, json_flag: bool, _data_dir: Option<PathBuf>) -> Result<()> {
    let lang = match a.lang.as_str() {
        "rs" | "rust" => Lang::Rust,
        "ts" => Lang::TypeScript,
        "tsx" => Lang::Tsx,
        "js" | "jsx" => Lang::JavaScript,
        "py" => Lang::Python,
        other => return Err(Error::Other(format!("unsupported lang: {other}"))),
    };
    let src = std::fs::read_to_string(&a.file)?;
    let hits = find(lang, &src, &a.pattern)?;
    let rows: Vec<Row> = hits
        .into_iter()
        .map(|(line, text)| Row { line, text })
        .collect();

    if json_flag {
        json::write(&rows)?;
    } else {
        let mut out = std::io::stdout().lock();
        for r in &rows {
            writeln!(out, "{}:{}  {}", a.file.display(), r.line, r.text)?;
        }
    }
    Ok(())
}
