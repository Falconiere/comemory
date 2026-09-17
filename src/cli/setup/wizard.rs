//! The cliclack wizard for `comemory setup`.
//!
//! Deliberately the thinnest file in the change. It holds no decisions —
//! `api::setup::plan` made them all — so there is nothing here to test
//! without a pty, and nothing here that a pty test would catch. It renders
//! what it is given and returns which steps the operator deselected.

use cliclack::{intro, multiselect, outro, outro_cancel};

use crate::api::setup::{Response, StepState};
use crate::prelude::*;

/// What the operator chose.
pub struct Selection {
    /// Step ids to add to the request's `skip` list.
    pub skipped: Vec<String>,
}

/// Offer every pending step and return the deselected ones.
///
/// `Ok(None)` means the operator cancelled (Esc / Ctrl-C) and nothing should
/// be applied. When no step is pending there is nothing to offer, so this
/// says so and returns an empty selection rather than building a multiselect
/// with no items — which cliclack rejects with `InvalidInput`.
///
/// # Errors
/// [`Error::Io`] for a genuine terminal failure. Cancellation is not one:
/// cliclack reports it as `ErrorKind::Interrupted`, which becomes
/// `Ok(None)`.
pub fn select(planned: &Response) -> Result<Option<Selection>> {
    let pending: Vec<&crate::api::setup::Step> = planned
        .steps
        .iter()
        .filter(|step| matches!(step.state, StepState::Pending))
        .collect();

    intro("comemory setup")?;
    report_context(planned)?;

    if pending.is_empty() {
        outro("Nothing to do — everything here is already set up.")?;
        return Ok(None);
    }

    let mut prompt = multiselect("What should I set up?").required(false);
    for step in &pending {
        prompt = prompt.item(step.id, step.title.as_str(), step.detail.as_str());
    }
    let chosen = match prompt.interact() {
        Ok(chosen) => chosen,
        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
            outro_cancel("Cancelled — nothing was applied.")?;
            return Ok(None);
        }
        Err(error) => return Err(Error::Io(error)),
    };

    Ok(Some(Selection {
        skipped: pending
            .iter()
            .map(|step| step.id)
            .filter(|id| !chosen.contains(id))
            .map(str::to_string)
            .collect(),
    }))
}

/// Show what was detected before asking anything, so the choice is informed.
fn report_context(planned: &Response) -> Result<()> {
    cliclack::log::remark(&planned.data_dir)?;
    for step in &planned.steps {
        if let StepState::Unavailable { reason } = &step.state {
            cliclack::log::warning(format!("{} — {reason}", step.title))?;
        }
    }
    Ok(())
}
