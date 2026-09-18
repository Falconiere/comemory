//! `comemory setup` — one command that takes a machine or a repo from
//! "binary installed" to "memory + code search working in my agent". The
//! detect/plan/apply engine lives in `domains::integrations::setup` (Binding
//! Rule 1); this
//! wrapper owns the argument shape, the mode decision, and rendering.

use std::io::IsTerminal as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::load_config;
use crate::cli::output::json;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::integrations;
use crate::prelude::*;
use crate::utilities::context::Ctx;
use crate::utilities::id_list::csv_unique;

/// TTY rendering of a finished plan.
pub mod render;
/// The interactive wizard.
pub mod wizard;

/// Example invocations shown at the bottom of `comemory setup --help`.
pub const EXAMPLES: &str = "\
Examples:
  # Interactive: pick what to enable here
  comemory setup

  # Non-interactive: apply everything this machine can
  comemory setup --yes

  # See the plan without changing anything
  comemory setup --dry-run --json

  # Just the git hooks, in another repo
  comemory setup --yes --only git-hooks --repo /path/to/repo";

/// Arguments to `comemory setup`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Apply every pending step without prompting.
    #[arg(long)]
    pub yes: bool,
    /// Report the plan and change nothing.
    #[arg(long, conflicts_with = "yes")]
    pub dry_run: bool,
    /// Comma-separated step ids to run; every other step is skipped.
    #[arg(long)]
    pub only: Option<String>,
    /// Comma-separated step ids to skip.
    #[arg(long)]
    pub skip: Option<String>,
    /// Restrict the agent-host step to one host (`claude` or `codex`).
    #[arg(long)]
    pub host: Option<String>,
    /// Repo root for the repo-scoped steps. Defaults to the working directory.
    #[arg(long)]
    pub repo: Option<PathBuf>,
}

/// How this invocation should behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Run the cliclack wizard, then apply what was selected.
    Wizard,
    /// Apply every pending step with no prompting.
    Apply,
    /// Report the plan and change nothing.
    Plan,
}

impl Mode {
    /// Whether this mode applies the plan.
    fn applies(self) -> bool {
        matches!(self, Self::Wizard | Self::Apply)
    }
}

/// What the operator asked for. `--dry-run` and `--yes` are declared as
/// conflicting in [`Args`], so the two flags can never both be set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Neither flag: ask, if we can.
    Ask,
    /// `--dry-run`: report and change nothing.
    Report,
    /// `--yes`: apply without asking.
    ApplyAll,
}

impl Intent {
    /// Read the intent off the parsed arguments.
    fn of(args: &Args) -> Self {
        match (args.dry_run, args.yes) {
            (true, _) => Self::Report,
            (_, true) => Self::ApplyAll,
            _ => Self::Ask,
        }
    }
}

/// Whether this invocation can put a prompt in front of a human.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompting {
    /// Stdin is a terminal and the caller did not ask for JSON.
    Available,
    /// Piped, redirected, or `--json`.
    Unavailable,
}

impl Prompting {
    /// Decide from the global `--json` flag and stdin.
    fn of(json_flag: bool, stdin_is_tty: bool) -> Self {
        if json_flag || !stdin_is_tty {
            Self::Unavailable
        } else {
            Self::Available
        }
    }
}

/// Decide the mode.
///
/// An explicit intent is honored as given. Without one, a terminal gets the
/// wizard and anything else falls back to reporting the plan — decided here
/// rather than letting cliclack surface its own bare `NotConnected`, so a
/// piped or CI invocation gets our message and a zero exit instead of a
/// failure.
pub fn mode(intent: Intent, prompting: Prompting) -> Mode {
    match intent {
        Intent::Report => Mode::Plan,
        Intent::ApplyAll => Mode::Apply,
        Intent::Ask => match prompting {
            Prompting::Available => Mode::Wizard,
            Prompting::Unavailable => Mode::Plan,
        },
    }
}

/// Run `comemory setup`.
///
/// # Errors
/// [`Error::Usage`] for an unknown step id or host, propagated from
/// `domains::integrations::setup::run` before anything is probed. A step that
/// fails while being
/// applied does not abort the run: the whole summary is rendered first, and
/// only then does this return [`Error::Unavailable`] so the process exits
/// non-zero (69) with the failure already on screen.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let mode = mode(
        Intent::of(&a),
        Prompting::of(json_flag, std::io::stdin().is_terminal()),
    );
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let mut req = integrations::setup::Request {
        repo: a.repo.map(|p| p.display().to_string()),
        host: a.host,
        only: a.only.as_deref().map(csv_unique).unwrap_or_default(),
        skip: a.skip.as_deref().map(csv_unique).unwrap_or_default(),
        apply: false,
    };

    if mode == Mode::Wizard {
        // Plan first so the wizard has something real to offer, then let the
        // operator narrow it before anything is written.
        let planned = integrations::setup::run(&mut ctx, clone_request(&req))?;
        let Some(selection) = wizard::select(&planned)? else {
            return Ok(());
        };
        req.skip = selection.skipped;
    }
    req.apply = mode.applies();

    let resp = integrations::setup::run(&mut ctx, req)?;
    if json_flag {
        json::write(&resp)?;
    } else {
        render::summary(&mut std::io::stdout().lock(), &resp, mode)?;
    }
    finish(&resp)
}

/// Copy a request for the wizard's planning pass. `Request` is deliberately
/// not `Clone` — it is a deserialized API input, not a value type — so the
/// one place that needs a second copy spells it out.
fn clone_request(req: &integrations::setup::Request) -> integrations::setup::Request {
    integrations::setup::Request {
        repo: req.repo.clone(),
        host: req.host.clone(),
        only: req.only.clone(),
        skip: req.skip.clone(),
        apply: false,
    }
}

/// Turn any applied-step failures into the command's exit status, after the
/// summary has already been written.
fn finish(resp: &integrations::setup::Response) -> Result<()> {
    if resp.failed == 0 {
        return Ok(());
    }
    Err(Error::Unavailable(format!(
        "setup: {} step(s) failed: {}",
        resp.failed,
        resp.failed_ids().join(", ")
    )))
}
