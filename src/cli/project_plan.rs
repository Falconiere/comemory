//! `comemory project plan show <PROJECT_ID>` (#335): the flags of the plan
//! verb and the TTY view of `domains::projects::plan`, a thin shell that
//! `project.rs` runs under the local operator's envelope. `ShowArgs` is the
//! id both `Show` and `Plan::Show` carry — `project.rs` reaches for it here.
use std::io::Write;

use clap::{Args as ClapArgs, Subcommand};

use crate::cli::project::emit;
use crate::domains::projects::authority::{self, Envelope};
use crate::domains::projects::plan::{self, PlanView};
use crate::prelude::*;
use crate::utilities::context::Ctx;

/// Args for `project show` and `project plan show` — both share the same id
/// flag and nothing else.
#[derive(ClapArgs, Debug)]
pub struct ShowArgs {
    /// The project's UUID.
    pub id: String,
}

/// Args for `project plan` — nested verb required.
#[derive(ClapArgs, Debug)]
pub struct PlanArgs {
    /// Nested verb.
    #[command(subcommand)]
    pub cmd: PlanCmd,
}

/// `project plan` verbs.
#[derive(Subcommand, Debug)]
pub enum PlanCmd {
    /// Show the plan at the current version: live milestones, work items,
    /// criteria of both levels and dependencies.
    Show(ShowArgs),
}

/// Run one `project plan` verb under the local operator's envelope.
pub fn run(ctx: &mut Ctx<'_>, operator: &Envelope, json_flag: bool, cmd: PlanCmd) -> Result<()> {
    match cmd {
        PlanCmd::Show(s) => {
            let resp = authority::run(ctx, operator, plan::Request { id: s.id })?;
            emit(json_flag, &resp, |out| render(out, &resp.plan))
        }
    }
}

/// The plan version, then one line per milestone, work item, criterion and
/// dependency.
pub fn render(out: &mut dyn Write, p: &PlanView) -> std::io::Result<()> {
    writeln!(out, "plan          v{} of {}", p.plan_version, p.project_id)?;
    for m in &p.milestones {
        let (status, date) = (&m.status, &m.target_date);
        writeln!(out, "milestone     [{status}] {}  {date}  {}", m.name, m.id)?;
    }
    for w in &p.work_items {
        let (number, status) = (w.number, &w.status);
        writeln!(
            out,
            "item          #{number} [{status}] {}  {}",
            w.title, w.id
        )?;
    }
    for c in &p.criteria {
        let scope = c.work_item_id.as_deref().unwrap_or("project");
        let (resolution, text) = (&c.criterion.resolution, &c.criterion.description);
        writeln!(out, "criterion     [{resolution}] {text}  ({scope})")?;
    }
    for d in &p.dependencies {
        writeln!(out, "blocks        {} -> {}", d.blocker_id, d.blocked_id)?;
    }
    let empty = p.milestones.is_empty() && p.work_items.is_empty();
    if empty && p.criteria.is_empty() && p.dependencies.is_empty() {
        writeln!(out, "no committed plan entities")?;
    }
    Ok(())
}
