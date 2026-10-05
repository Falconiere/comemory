//! `comemory project proposal submit|list|show` — the plan-proposal verbs
//! (#336), thin shells around `domains::projects::{propose, proposals}` run
//! under the local operator's envelope. Clap argument ids equal the core
//! request's serde names, so the MCP parity probe maps `project_propose`
//! onto them; `--operations-file` is the CLI's file-body convenience that
//! collapses into `operations`.
//!
//! Submit assembles the platform's JSON body and parses it through
//! `utilities::project_body`, so a malformed operation list is refused here
//! exactly as `POST …/proposals` refuses it.

use std::io::Read;
use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};
use serde_json::{Value, json};

use crate::cli::project::emit;
use crate::cli::project_transfer;
use crate::domains::projects::authority::{self, Envelope};
use crate::domains::projects::proposal_view::ProposalView;
use crate::domains::projects::proposals::{ListRequest, ListResponse, ShowRequest};
use crate::domains::projects::propose;
use crate::prelude::*;
use crate::utilities::context::Ctx;
use crate::utilities::project_body::from_value;
use crate::utilities::project_error::ProjectError;
use crate::utilities::uuid;

/// Args for `project proposal` — nested verb required.
#[derive(ClapArgs, Debug)]
pub struct ProposalArgs {
    /// Nested verb.
    #[command(subcommand)]
    pub cmd: ProposalCmd,
}

/// `project proposal` verbs.
#[derive(Subcommand, Debug)]
pub enum ProposalCmd {
    /// Submit an immutable plan proposal for review against a plan version.
    Submit(SubmitArgs),
    /// List a project's proposals newest first, one keyset page at a time.
    List(ListArgs),
    /// Show one proposal of a project.
    Show(ShowArgs),
}

/// Args for `project proposal submit`.
#[derive(ClapArgs, Debug)]
#[command(group(clap::ArgGroup::new("source").required(true).args(["operations", "operationsFile"])))]
pub struct SubmitArgs {
    /// The project's UUID.
    #[arg(id = "projectId", value_name = "PROJECT_ID")]
    pub project_id: String,
    /// The plan version the operations were written against.
    #[arg(long = "base-plan-version", id = "basePlanVersion")]
    pub base_plan_version: i64,
    /// The operations as a JSON array (1–200 typed plan operations).
    #[arg(long)]
    pub operations: Option<String>,
    /// Read the operations JSON array from this file (`-` for stdin).
    #[arg(long = "operations-file", id = "operationsFile", value_name = "PATH")]
    pub operations_file: Option<PathBuf>,
    /// Why the change is proposed (1–4000 characters).
    #[arg(long)]
    pub rationale: String,
    /// An assumption; repeat for more (up to 50).
    #[arg(long = "assumption", id = "assumptions")]
    pub assumptions: Vec<String>,
    /// A risk; repeat for more (up to 50).
    #[arg(long = "risk", id = "risks")]
    pub risks: Vec<String>,
    /// Retry key (1–200 UTF-16 units): rerunning with the same key and body
    /// prints the first answer and writes nothing. Minted when omitted.
    #[arg(long = "idempotency-key", id = "idempotencyKey")]
    pub idempotency_key: Option<String>,
}

/// Args for `project proposal list`.
#[derive(ClapArgs, Debug)]
pub struct ListArgs {
    /// The project's UUID.
    #[arg(id = "projectId", value_name = "PROJECT_ID")]
    pub project_id: String,
    /// Only this state: pending, approved, changes_requested, rejected, superseded.
    #[arg(long)]
    pub state: Option<String>,
    /// The previous page's `nextCursor`.
    #[arg(long)]
    pub cursor: Option<String>,
    /// Page size, 1–100 (default 20).
    #[arg(long)]
    pub limit: Option<i64>,
}

/// Args for `project proposal show`.
#[derive(ClapArgs, Debug)]
pub struct ShowArgs {
    /// The project's UUID.
    #[arg(id = "projectId", value_name = "PROJECT_ID")]
    pub project_id: String,
    /// The proposal's UUID.
    #[arg(id = "proposalId", value_name = "PROPOSAL_ID")]
    pub proposal_id: String,
}

/// Run one `project proposal` verb as `operator`.
pub fn run(
    cmd: ProposalCmd,
    ctx: &mut Ctx<'_>,
    operator: &Envelope,
    json_flag: bool,
) -> Result<()> {
    match cmd {
        ProposalCmd::Submit(s) => {
            let resp = authority::run(ctx, operator, submit_request(s)?)?;
            project_transfer::warn_local_only(&resp.warnings)?;
            emit(json_flag, &resp, |out| render(out, &resp.proposal))
        }
        ProposalCmd::List(l) => {
            let req = ListRequest {
                project_id: l.project_id,
                limit: l.limit,
                cursor: l.cursor,
                state: l.state,
            };
            let resp = authority::run(ctx, operator, req)?;
            emit(json_flag, &resp, |out| render_page(out, &resp))
        }
        ProposalCmd::Show(s) => {
            let req = ShowRequest {
                project_id: s.project_id,
                proposal_id: s.proposal_id,
            };
            let resp = authority::run(ctx, operator, req)?;
            emit(json_flag, &resp, |out| render(out, &resp.proposal))
        }
    }
}

/// The platform's body from the flags, parsed as `POST …/proposals` parses
/// it; the key minted when absent.
fn submit_request(s: SubmitArgs) -> Result<propose::Request> {
    let text = match (s.operations, s.operations_file) {
        (Some(text), _) => text,
        (None, Some(path)) => read_operations(&path)?,
        (None, None) => String::new(),
    };
    let operations: Value = serde_json::from_str(&text)
        .map_err(|_| Error::from(ProjectError::invalid_field("operations", "invalid")))?;
    let idempotency_key = match s.idempotency_key {
        Some(key) => key,
        None => uuid::new_v4()?,
    };
    let raw = json!({
        "projectId": s.project_id,
        "idempotencyKey": idempotency_key,
        "basePlanVersion": s.base_plan_version,
        "operations": operations,
        "rationale": s.rationale,
        "assumptions": s.assumptions,
        "risks": s.risks,
    });
    from_value(raw)
}

/// The operations text at `path`, or stdin for `-`.
fn read_operations(path: &std::path::Path) -> Result<String> {
    if path.as_os_str() == "-" {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        return Ok(text);
    }
    Ok(std::fs::read_to_string(path)?)
}

/// One proposal: its identity and state, then one line per operation,
/// assumption and risk.
fn render(out: &mut dyn std::io::Write, p: &ProposalView) -> std::io::Result<()> {
    writeln!(out, "proposal      {}", p.id)?;
    writeln!(out, "project       {}", p.project_id)?;
    writeln!(
        out,
        "state         {} (base plan v{})",
        p.state, p.base_plan_version
    )?;
    let (kind, who) = (&p.proposer_principal_type, &p.proposer_principal_id);
    writeln!(out, "proposer      {who} ({kind})")?;
    writeln!(out, "rationale     {}", p.rationale)?;
    for operation in &p.operations {
        let text = serde_json::to_string(operation).map_err(std::io::Error::other)?;
        writeln!(out, "operation     {text}")?;
    }
    for a in &p.assumptions {
        writeln!(out, "assumption    {a}")?;
    }
    for r in &p.risks {
        writeln!(out, "risk          {r}")?;
    }
    if let Some(review) = &p.review {
        writeln!(
            out,
            "review        {} by {}",
            review.decision, review.reviewer_id
        )?;
    }
    writeln!(out, "created       {}", p.created_at)
}

/// One line per proposal, then the next page's cursor when there is one.
fn render_page(out: &mut dyn std::io::Write, page: &ListResponse) -> std::io::Result<()> {
    if page.proposals.is_empty() {
        writeln!(out, "no proposals")?;
    }
    for p in &page.proposals {
        let (state, ops) = (&p.state, p.operations.len());
        writeln!(
            out,
            "{state:<17} v{:<4} {ops:>3} ops  {}  {}",
            p.base_plan_version, p.id, p.created_at
        )?;
    }
    if let Some(cursor) = &page.next_cursor {
        writeln!(out, "next page: --cursor {cursor}")?;
    }
    Ok(())
}
