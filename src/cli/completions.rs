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
  # fish (autoloaded from this path)
  qwick-memory completions fish > ~/.config/fish/completions/qwick-memory.fish

  # zsh (homebrew site-functions path)
  qwick-memory completions zsh > \"$(brew --prefix)/share/zsh/site-functions/_qwick-memory\"

  # bash (homebrew bash-completion.d)
  qwick-memory completions bash > \"$(brew --prefix)/etc/bash_completion.d/qwick-memory\"

  # NOTE: scripts/install.sh writes these automatically by default.";

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
