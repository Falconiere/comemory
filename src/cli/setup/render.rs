//! TTY rendering for `comemory setup`.
//!
//! Everything writes into an `impl Write` rather than straight to stdout, so
//! the summary is snapshot-tested from a crate-root test instead of being
//! proven only by eye.

use std::io::Write;

use super::Mode;
use crate::cli::output::tty;
use crate::domains::integrations::setup::{Response, Step, StepState};
use crate::prelude::*;

/// The marker shown beside each step.
fn marker(state: &StepState) -> &'static str {
    match state {
        StepState::Satisfied | StepState::Applied { .. } => "✔",
        StepState::Pending => "◻",
        StepState::Skipped => "–",
        StepState::Unavailable { .. } => "!",
        StepState::Failed { .. } => "✘",
    }
}

/// The trailing clause for a step, if it has one.
fn note(state: &StepState) -> Option<String> {
    match state {
        StepState::Unavailable { reason } => Some(reason.clone()),
        StepState::Failed { error } => Some(error.clone()),
        StepState::Applied { detail } => Some(detail.clone()),
        _ => None,
    }
}

/// Write one step line: `  ✔ Title — detail`.
fn step_line(out: &mut impl Write, step: &Step) -> Result<()> {
    let trailing = note(&step.state).unwrap_or_else(|| step.detail.clone());
    if trailing.is_empty() {
        writeln!(out, "  {} {}", marker(&step.state), step.title)?;
    } else {
        writeln!(
            out,
            "  {} {} {}",
            marker(&step.state),
            step.title,
            tty::dim(&format!("— {trailing}"))
        )?;
    }
    Ok(())
}

/// Write the whole summary: a header, one line per step, and a footer that
/// says what to do next.
///
/// # Errors
/// Propagates a write failure from `out`.
pub fn summary(out: &mut impl Write, resp: &Response, mode: Mode) -> Result<()> {
    tty::write_header(out, header(mode))?;
    writeln!(out, "  {}", tty::dim(&location(resp)))?;
    for step in &resp.steps {
        step_line(out, step)?;
    }
    footer(out, resp, mode)
}

/// The one-line header for this mode.
fn header(mode: Mode) -> &'static str {
    match mode {
        Mode::Plan => "comemory setup — plan",
        Mode::Apply | Mode::Wizard => "comemory setup",
    }
}

/// `data dir · repo` context line.
fn location(resp: &Response) -> String {
    match &resp.repo {
        Some(repo) => format!("{} · repo {}", resp.data_dir, repo.label),
        None => format!("{} · not in a git repository", resp.data_dir),
    }
}

/// Write the closing line: what was done, or what to run next.
fn footer(out: &mut impl Write, resp: &Response, mode: Mode) -> Result<()> {
    writeln!(out)?;
    if resp.failed > 0 {
        writeln!(
            out,
            "{} step(s) failed: {}",
            resp.failed,
            resp.failed_ids().join(", ")
        )?;
        return Ok(());
    }
    if mode == Mode::Plan {
        let pending = resp
            .steps
            .iter()
            .filter(|s| matches!(s.state, StepState::Pending))
            .count();
        if pending == 0 {
            writeln!(out, "Nothing to do — everything here is already set up.")?;
        } else {
            writeln!(
                out,
                "{pending} step(s) would run. Apply them with: comemory setup --yes"
            )?;
        }
        return Ok(());
    }
    if resp.applied == 0 {
        writeln!(out, "Nothing to do — everything here is already set up.")?;
    } else {
        writeln!(
            out,
            "Done — {} step(s) applied. Try: comemory find \"how does auth work\"",
            resp.applied
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/render.rs"]
mod tests;
