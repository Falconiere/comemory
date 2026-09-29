//! `comemory project` — engine-owned project management (epic #261). Each
//! verb is a thin shell around a `domains::projects` core: this file owns the
//! flags and the TTY rendering, nothing else. The CLI runs every core under
//! the local operator's envelope: a `user` at `owner` tier (#315).
//!
//! Clap argument ids equal the core request's serde names (`keyPrefix`,
//! `includeArchived`, …), so the MCP parity probe maps a flag onto the field
//! an agent sends.

use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};

use crate::cli::load_config;
use crate::cli::output::json;
use crate::cli::project_transfer::{self, ExportArgs, ImportArgs};
use crate::cli::{project_activity, project_lifecycle};
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::projects::authority::{self, Envelope};
use crate::domains::projects::changes::{self, ChangeFrame};
use crate::domains::projects::lifecycle::Kind;
use crate::domains::projects::plan::{self, PlanView};
use crate::domains::projects::view::ProjectView;
use crate::domains::projects::{create, lifecycle, list, show};
use crate::prelude::*;
use crate::utilities::context::Ctx;
use crate::utilities::uuid;

const EXAMPLES: &str = "\
Examples:
  # Charter a project offline (no account needed)
  comemory project create --name 'Ship offline projects' --key-prefix SHIP \\
    --outcome 'Projects work without a cloud account' \\
    --success-criterion 'A project is created offline' --repository falconiere/comemory

  # Retry-safe: rerunning with the same key prints the first answer again
  comemory project create --name 'Ship offline projects' --key-prefix SHIP \\
    --outcome 'Projects work without a cloud account' --idempotency-key ship-2026-09

  # Read it back
  comemory project show 0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f --json

  # Read its committed plan: milestones, work items, criteria, dependencies
  comemory project plan show 0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f --json

  # Page through active projects, newest first
  comemory project list --status active --limit 10
  comemory project list --cursor '<nextCursor from the previous page>'

  # Pause an active project, then resume it (each bumps its version)
  comemory project pause 0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f --expected-version 3 \\
    --reason 'Waiting on the design review'
  comemory project resume 0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f --expected-version 4

  # Archive it out of the default list, and restore it
  comemory project archive 0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f --expected-version 5
  comemory project restore 0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f --expected-version 6

  # Walk one project's activity, oldest first
  comemory project activity 0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f --order asc --limit 50

  # Poll the body-free change feed from a cursor
  comemory project changes --after 0 --limit 100 --json

  # Move one project to another data directory under the same ids
  comemory project export 0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f --output ship.json
  comemory --data-dir ~/other project import ship.json";

/// Top-level `project` args — nested verb required.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Nested verb.
    #[command(subcommand)]
    pub cmd: ProjectCmd,
}

/// `project` verbs.
#[derive(Subcommand, Debug)]
pub enum ProjectCmd {
    /// Charter a draft project and record its `project.created` event.
    Create(CreateArgs),
    /// Show one project's charter by id.
    Show(ShowArgs),
    /// List projects newest first, one keyset page at a time.
    List(ListArgs),
    /// Archive a project: hide it from the default list, keep its history.
    Archive(project_lifecycle::Args),
    /// Restore an archived project that is not completed or canceled.
    Restore(project_lifecycle::Args),
    /// Pause an active project, with a reason.
    Pause(project_lifecycle::Args),
    /// Resume a paused project.
    Resume(project_lifecycle::Args),
    /// Page one project's activity log, newest first or oldest first.
    Activity(project_activity::Args),
    /// Read the body-free change feed: one frame per committed mutation.
    Changes(ChangesArgs),
    /// Write one project, with every row it owns that travels, as a transfer
    /// bundle (receipts stay behind).
    Export(ExportArgs),
    /// Import a transfer bundle under the same ids: an identical copy is
    /// unchanged, a different one skipped, and nothing is ever overwritten.
    Import(ImportArgs),
    /// Read a project's committed plan.
    Plan(PlanArgs),
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

/// Args for `project create`.
#[derive(ClapArgs, Debug)]
pub struct CreateArgs {
    /// Display name (1–120 characters).
    #[arg(long)]
    pub name: String,
    /// Work-item key prefix: 2–10 of `A-Z0-9`, starting with a letter, unique.
    #[arg(long = "key-prefix", id = "keyPrefix")]
    pub key_prefix: String,
    /// The finite outcome the project exists to reach (1–2000 characters).
    #[arg(long)]
    pub outcome: String,
    /// A project-level success criterion; repeat for more (up to 50).
    #[arg(long = "success-criterion", id = "successCriteria")]
    pub success_criteria: Vec<String>,
    /// A constraint; repeat for more (up to 50).
    #[arg(long = "constraint", id = "constraints")]
    pub constraints: Vec<String>,
    /// A non-goal; repeat for more (up to 50).
    #[arg(long = "non-goal", id = "nonGoals")]
    pub non_goals: Vec<String>,
    /// A canonical `owner/name` repository; repeat for more (up to 50).
    #[arg(long = "repository", id = "repositories")]
    pub repositories: Vec<String>,
    /// The lead's principal id (default: the local operator).
    #[arg(long = "lead", id = "leadUserId")]
    pub lead_user_id: Option<String>,
    /// Target date: `YYYY-MM-DD` (UTC midnight) or an RFC 3339 timestamp.
    #[arg(long = "target-date", id = "targetDate")]
    pub target_date: Option<String>,
    /// Use this UUID as the project id instead of minting one.
    #[arg(long)]
    pub id: Option<String>,
    /// Retry key (1–200 UTF-16 units): rerunning with the same key and flags
    /// prints the first answer and writes nothing. A fresh one is minted when
    /// omitted, so only a run that names its key is retry-safe.
    #[arg(long = "idempotency-key", id = "idempotencyKey")]
    pub idempotency_key: Option<String>,
}

/// Args for `project show`.
#[derive(ClapArgs, Debug)]
pub struct ShowArgs {
    /// The project's UUID.
    pub id: String,
}

/// Args for `project list`.
#[derive(ClapArgs, Debug)]
pub struct ListArgs {
    /// Only this status: draft, planning, active, paused, completed, canceled.
    #[arg(long)]
    pub status: Option<String>,
    /// Only this health: unknown, on_track, at_risk, off_track.
    #[arg(long)]
    pub health: Option<String>,
    /// Include archived projects.
    #[arg(long = "include-archived", id = "includeArchived")]
    pub include_archived: bool,
    /// The previous page's `nextCursor`.
    #[arg(long)]
    pub cursor: Option<String>,
    /// Page size, 1–100 (default 20).
    #[arg(long)]
    pub limit: Option<i64>,
}

/// Args for `project changes`.
#[derive(ClapArgs, Debug)]
pub struct ChangesArgs {
    /// Frames after this `seq` (default 0, the start).
    #[arg(long)]
    pub after: Option<i64>,
    /// Page size, 1–1000 (default 100).
    #[arg(long)]
    pub limit: Option<i64>,
}

/// Run one `project` verb.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    // A fresh data directory is a valid place to charter the first project.
    paths.ensure_dirs()?;
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let operator = Envelope::local_operator();
    match a.cmd {
        ProjectCmd::Create(c) => {
            let resp = authority::run(&mut ctx, &operator, create_request(c)?)?;
            emit(json_flag, &resp, |out| render_project(out, &resp.project))
        }
        ProjectCmd::Show(s) => {
            let resp = authority::run(&mut ctx, &operator, show::Request { id: s.id })?;
            emit(json_flag, &resp, |out| {
                render_project(out, &resp.project)?;
                project_transfer::render_binding(out, resp.transfer.as_ref())
            })
        }
        ProjectCmd::List(l) => {
            let resp = authority::run(&mut ctx, &operator, list_request(l))?;
            emit(json_flag, &resp, |out| render_page(out, &resp))
        }
        ProjectCmd::Archive(l) => lifecycle(&mut ctx, json_flag, l.request(Kind::Archive)?),
        ProjectCmd::Restore(l) => lifecycle(&mut ctx, json_flag, l.request(Kind::Restore)?),
        ProjectCmd::Pause(l) => lifecycle(&mut ctx, json_flag, l.request(Kind::Pause)?),
        ProjectCmd::Resume(l) => lifecycle(&mut ctx, json_flag, l.request(Kind::Resume)?),
        ProjectCmd::Activity(a) => {
            let resp = authority::run(&mut ctx, &operator, a.request())?;
            emit(json_flag, &resp, |out| project_activity::render(out, &resp))
        }
        ProjectCmd::Changes(c) => {
            let req = changes::Request {
                after: c.after,
                limit: c.limit,
            };
            let frames = authority::run(&mut ctx, &operator, req)?;
            emit(json_flag, &frames, |out| render_changes(out, &frames))
        }
        ProjectCmd::Plan(PlanArgs {
            cmd: PlanCmd::Show(s),
        }) => {
            let resp = authority::run(&mut ctx, &operator, plan::Request { id: s.id })?;
            emit(json_flag, &resp, |out| render_plan(out, &resp.plan))
        }
        ProjectCmd::Export(e) => project_transfer::export(&mut ctx, &operator, e, json_flag),
        ProjectCmd::Import(i) => project_transfer::import(&mut ctx, &operator, i, json_flag),
    }
}

/// Run one lifecycle verb as the local operator and print the project.
fn lifecycle(ctx: &mut Ctx<'_>, json_flag: bool, req: lifecycle::Request) -> Result<()> {
    let resp = authority::run(ctx, &Envelope::local_operator(), req)?;
    project_transfer::warn_local_only(&resp.warnings)?;
    emit(json_flag, &resp, |out| render_project(out, &resp.project))
}

/// The core request for `project create`, its key minted when absent.
fn create_request(c: CreateArgs) -> Result<create::Request> {
    let idempotency_key = match c.idempotency_key {
        Some(key) => key,
        None => uuid::new_v4()?,
    };
    Ok(create::Request {
        id: c.id,
        workspace_id: None,
        idempotency_key,
        name: c.name,
        key_prefix: c.key_prefix,
        outcome: c.outcome,
        success_criteria: c.success_criteria,
        constraints: c.constraints,
        non_goals: c.non_goals,
        repositories: c.repositories,
        lead_user_id: c.lead_user_id,
        target_date: c.target_date,
    })
}

/// The core request for `project list`.
fn list_request(l: ListArgs) -> list::Request {
    list::Request {
        limit: l.limit,
        cursor: l.cursor,
        status: l.status,
        health: l.health,
        include_archived: Some(l.include_archived),
    }
}

/// `--json` prints `resp` verbatim; otherwise `tty` renders it.
fn emit<T: serde::Serialize>(
    json_flag: bool,
    resp: &T,
    tty: impl FnOnce(&mut dyn std::io::Write) -> std::io::Result<()>,
) -> Result<()> {
    if json_flag {
        return json::write(resp);
    }
    let mut out = std::io::stdout().lock();
    Ok(tty(&mut out)?)
}

/// One project's charter, one field per line.
fn render_project(out: &mut dyn std::io::Write, p: &ProjectView) -> std::io::Result<()> {
    writeln!(out, "id            {}", p.id)?;
    writeln!(out, "key           {}", p.key_prefix)?;
    writeln!(out, "slug          {}", p.slug)?;
    writeln!(out, "name          {}", p.name)?;
    writeln!(out, "status        {} ({})", p.status, p.health)?;
    writeln!(out, "version       {}", p.version)?;
    let archived = p.archived_at.as_deref().unwrap_or("-");
    writeln!(out, "archived      {archived}")?;
    writeln!(out, "lead          {}", p.lead_user_id)?;
    writeln!(
        out,
        "target        {}",
        p.target_date.as_deref().unwrap_or("-")
    )?;
    writeln!(out, "outcome       {}", p.outcome)?;
    for c in &p.criteria {
        writeln!(out, "criterion     [{}] {}", c.resolution, c.description)?;
    }
    for c in &p.constraints {
        writeln!(out, "constraint    {c}")?;
    }
    for g in &p.non_goals {
        writeln!(out, "non-goal      {g}")?;
    }
    for r in &p.repositories {
        writeln!(out, "repository    {r}")?;
    }
    writeln!(out, "created       {}", p.created_at)
}

/// One line per project, then the next page's cursor when there is one.
fn render_page(out: &mut dyn std::io::Write, page: &list::Response) -> std::io::Result<()> {
    if page.projects.is_empty() {
        writeln!(out, "no projects")?;
    }
    for p in &page.projects {
        let (key, status, health) = (&p.key_prefix, &p.status, &p.health);
        writeln!(
            out,
            "{key:<10} {status:<9} {health:<9} {}  {}",
            p.id, p.name
        )?;
    }
    if let Some(cursor) = &page.next_cursor {
        writeln!(out, "next page: --cursor {cursor}")?;
    }
    Ok(())
}

/// The plan version, then one line per milestone, work item, criterion and
/// dependency.
fn render_plan(out: &mut dyn std::io::Write, p: &PlanView) -> std::io::Result<()> {
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

/// One line per frame, then the next read's cursor when there was a frame.
fn render_changes(out: &mut dyn std::io::Write, frames: &[ChangeFrame]) -> std::io::Result<()> {
    if frames.is_empty() {
        writeln!(out, "no changes")?;
    }
    for f in frames {
        writeln!(
            out,
            "{:<8} {:<8} {}  {}",
            f.seq, f.op, f.project_id, f.event_id
        )?;
    }
    if let Some(last) = frames.last() {
        writeln!(out, "next: --after {}", last.seq)?;
    }
    Ok(())
}
