//! `comemory install-hooks` — drop git hooks into a repo so commits/merges/
//! checkouts kick off `comemory index-code --incremental --quiet` in the
//! background.
//!
//! The hooks are intentionally minimal (`exec comemory … &`) so they don't slow
//! down interactive git operations. If `comemory` isn't on `$PATH` the hook
//! fails silently — git treats a missing executable as a hook error but the
//! `&` detaches before the exit code reaches git, so the commit still
//! completes cleanly.
//!
//! By default the command refuses to overwrite an existing hook file; pass
//! `--force` to replace whatever is there. This guards against trampling a
//! user's hand-written hook on first install.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::git_utils::install_hook;
use crate::output::json;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Install into the current repo
  comemory install-hooks

  # Install into a specific repo path
  comemory install-hooks --repo /path/to/repo

  # Overwrite any hand-written hooks
  comemory install-hooks --force";

/// Arguments to `comemory install-hooks`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Repo root to install hooks into. Defaults to the current working
    /// directory.
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,
    /// Overwrite existing hook files. Without this flag the command refuses
    /// to clobber a pre-existing `post-commit`/`post-merge`/`post-checkout`
    /// to avoid surprising users with hand-written hooks.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

/// Body written to each hook file. The trailing `&` detaches the indexer so
/// git's hook runner returns immediately.
///
/// `--repo` and `--path` are required by `comemory index-code`; we derive the
/// repo label from the working-tree directory name and the path from the repo
/// root via `git rev-parse --show-toplevel`. The `index-code` walker uses each
/// file's blob OID as the cursor, so re-running it on an unchanged tree is a
/// cheap no-op (the v0.2 replacement for the old `--incremental` flag).
const SCRIPT: &str = "#!/usr/bin/env bash\n\
                      ROOT=\"$(git rev-parse --show-toplevel 2>/dev/null)\"\n\
                      [ -z \"$ROOT\" ] && exit 0\n\
                      REPO=\"$(basename \"$ROOT\")\"\n\
                      ( comemory index-code --repo \"$REPO\" --path \"$ROOT\" >/dev/null 2>&1 & )\n\
                      exit 0\n";

/// Hooks we install. All three trigger an incremental reindex because each
/// can leave the working tree at a new HEAD: `post-commit` for new commits,
/// `post-merge` for fast-forward/merge updates, `post-checkout` for branch
/// switches and `git checkout <file>` (which can also touch working-tree
/// files we may want to re-embed).
const HOOKS: &[&str] = &["post-commit", "post-merge", "post-checkout"];

/// Install (or, with `--force`, overwrite) the three reindex hooks. On
/// success the human-readable line lists the hooks that were written; under
/// `--json` we emit a small object so callers can detect success
/// programmatically.
pub async fn run(a: Args, json_flag: bool, _data_dir: Option<PathBuf>) -> Result<()> {
    for hook in HOOKS {
        let target = a.repo.join(".git").join("hooks").join(hook);
        if target.exists() && !a.force {
            return Err(Error::Other(format!(
                "{} already exists; pass --force to overwrite",
                target.display()
            )));
        }
        install_hook(&a.repo, hook, SCRIPT)?;
    }
    if json_flag {
        json::write(&serde_json::json!({
            "installed": HOOKS,
            "repo": a.repo.display().to_string(),
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "installed {} hooks in {}",
            HOOKS.join(", "),
            a.repo.display()
        )?;
    }
    Ok(())
}
