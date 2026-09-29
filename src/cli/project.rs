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
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::projects::authority::{self, Envelope};
use crate::domains::projects::changes::{self, ChangeFrame};
use crate::domains::projects::view::ProjectView;
use crate::domains::projects::{create, list, show};
use crate::prelude::*;
use crate::utilities::context::Ctx;

const EXAMPLES: &str = "\
Examples:
  # Charter a project offline (no account needed)
  comemory project create --name 'Ship offline projects' --key-prefix SHIP \\
    --outcome 'Projects work without a cloud account' \\
    --success-criterion 'A project is created offline' --repository falconiere/comemory

  # Read it back
  comemory project show 0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f --json

  # Page through active projects, newest first
  comemory project list --status active --limit 10
  comemory project list --cursor '<nextCursor from the previous page>'

  # Poll the body-free change feed from a cursor
  comemory project changes --after 0 --limit 100 --json";

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
    /// Read the body-free change feed: one frame per committed mutation.
    Changes(ChangesArgs),
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
            let resp = authority::run(&mut ctx, &operator, create_request(c))?;
            emit(json_flag, &resp, |out| render_project(out, &resp.project))
        }
        ProjectCmd::Show(s) => {
            let resp = authority::run(&mut ctx, &operator, show::Request { id: s.id })?;
            emit(json_flag, &resp, |out| render_project(out, &resp.project))
        }
        ProjectCmd::List(l) => {
            let resp = authority::run(&mut ctx, &operator, list_request(l))?;
            emit(json_flag, &resp, |out| render_page(out, &resp))
        }
        ProjectCmd::Changes(c) => {
            let req = changes::Request {
                after: c.after,
                limit: c.limit,
            };
            let frames = authority::run(&mut ctx, &operator, req)?;
            emit(json_flag, &frames, |out| render_changes(out, &frames))
        }
    }
}

/// The core request for `project create`.
fn create_request(c: CreateArgs) -> create::Request {
    create::Request {
        id: c.id,
        workspace_id: None,
        name: c.name,
        key_prefix: c.key_prefix,
        outcome: c.outcome,
        success_criteria: c.success_criteria,
        constraints: c.constraints,
        non_goals: c.non_goals,
        repositories: c.repositories,
        lead_user_id: c.lead_user_id,
        target_date: c.target_date,
    }
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
