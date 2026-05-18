//! `qwick-memory completions <shell>` — emit a shell completion script on stdout.
//!
//! Wraps `clap_complete::generate` against the top-level `Cli` so completions
//! always reflect the current subcommand surface. The `--json` and
//! `--data-dir` globals are accepted but ignored: this subcommand only
//! produces shell script text.

use std::io;
use std::path::PathBuf;

use clap::{Args as ClapArgs, CommandFactory};
use clap_complete::{generate, Shell};

use crate::cli::Cli;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  qwick-memory completions fish > ~/.config/fish/completions/qwick-memory.fish
  qwick-memory completions zsh  > \"${fpath[1]}/_qwick-memory\"
  qwick-memory completions bash > /usr/local/etc/bash_completion.d/qwick-memory";

/// Arguments for `qwick-memory completions`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Shell to emit a completion script for.
    pub shell: Shell,
}

/// Emit the completion script for `a.shell` on stdout.
pub async fn run(a: Args, _json: bool, _data_dir: Option<PathBuf>) -> Result<()> {
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    let mut out = io::stdout().lock();
    generate(a.shell, &mut cmd, bin_name, &mut out);
    Ok(())
}
