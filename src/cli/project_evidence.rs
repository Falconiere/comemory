//! `comemory project evidence add|list` (#346): the flags and TTY views of
//! typed evidence, thin shells over `domains::projects::evidence_add` and
//! `evidence_page`; [`run`] executes them under the envelope `project.rs`
//! hands it, the local operator's. Clap argument ids equal the core requests' serde names, so the
//! MCP parity probe maps each `add` flag onto `project_evidence`.

use clap::{Args as ClapArgs, Subcommand};
use serde_json::Value;

use crate::cli::project::emit;
use crate::cli::project_transfer;
use crate::domains::projects::authority::{self, Envelope};
use crate::domains::projects::evidence::EvidenceView;
use crate::domains::projects::{evidence_add, evidence_page};
use crate::prelude::*;
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;
use crate::utilities::uuid;

/// Args for `project evidence` — nested verb required.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Nested verb.
    #[command(subcommand)]
    pub cmd: EvidenceCmd,
}

/// `project evidence` verbs.
#[derive(Subcommand, Debug)]
pub enum EvidenceCmd {
    /// Record typed evidence against a project or one of its work items.
    Add(AddArgs),
    /// Page a project's evidence newest first, filtered by kind, trust and
    /// work item.
    List(ListArgs),
}

/// Args for `project evidence add`.
#[derive(ClapArgs, Debug)]
pub struct AddArgs {
    /// The project's UUID.
    #[arg(id = "projectId", value_name = "PROJECT_ID")]
    pub project_id: String,
    /// commit, pull_request, test_run, deployment, session, decision, memory
    /// or external_url.
    #[arg(long)]
    pub kind: String,
    /// The system the evidence came from (1–120 characters).
    #[arg(long)]
    pub source: String,
    /// Attach to this work item instead of the project itself.
    #[arg(long = "work-item", id = "workItemId")]
    pub work_item_id: Option<String>,
    /// Its id in the source system (1–256 characters): a pull-request
    /// number, a check-run name, a memory id, …
    #[arg(long = "external-id", id = "externalId")]
    pub external_id: Option<String>,
    /// An absolute URL (at most 2048 characters).
    #[arg(long)]
    pub url: Option<String>,
    /// Canonical `owner/name`; must be one of the project's repositories.
    #[arg(long)]
    pub repo: Option<String>,
    /// Hex commit id (1–256 characters).
    #[arg(long = "commit-sha", id = "commitSha")]
    pub commit_sha: Option<String>,
    /// A JSON object stored with the claim (16 KiB encoded).
    #[arg(long)]
    pub metadata: Option<String>,
    /// A criterion of this project the evidence may satisfy; repeat for more
    /// (up to 20).
    #[arg(long = "criterion", id = "criterionIds")]
    pub criterion_ids: Vec<String>,
    /// Retry key (1–200 UTF-16 units): rerunning with the same key and flags
    /// prints the first answer and writes nothing. A fresh one is minted when
    /// omitted, so only a run that names its key is retry-safe.
    #[arg(long = "idempotency-key", id = "idempotencyKey")]
    pub idempotency_key: Option<String>,
}

impl AddArgs {
    /// The core request these flags name, its key minted when absent and
    /// `--metadata` parsed as a JSON object (`400` on `metadata` otherwise).
    pub fn request(self) -> Result<evidence_add::Request> {
        let metadata = match self.metadata.as_deref().map(serde_json::from_str) {
            None => None,
            Some(Ok(Value::Object(map))) => Some(map),
            Some(_) => return Err(ProjectError::invalid_field("metadata", "invalid").into()),
        };
        let idempotency_key = match self.idempotency_key {
            Some(key) => key,
            None => uuid::new_v4()?,
        };
        Ok(evidence_add::Request {
            project_id: self.project_id,
            idempotency_key,
            work_item_id: self.work_item_id,
            kind: self.kind,
            source: self.source,
            external_id: self.external_id,
            url: self.url,
            repo: self.repo,
            commit_sha: self.commit_sha,
            metadata,
            criterion_ids: self.criterion_ids,
        })
    }
}

/// Args for `project evidence list`.
#[derive(ClapArgs, Debug)]
pub struct ListArgs {
    /// The project's UUID.
    #[arg(id = "projectId", value_name = "PROJECT_ID")]
    pub project_id: String,
    /// Only this kind.
    #[arg(long)]
    pub kind: Option<String>,
    /// Only this trust: verified, self_reported, pending or invalid.
    #[arg(long)]
    pub trust: Option<String>,
    /// Only evidence on this work item.
    #[arg(long = "work-item", id = "workItemId")]
    pub work_item_id: Option<String>,
    /// The previous page's `nextCursor`.
    #[arg(long)]
    pub cursor: Option<String>,
    /// Page size, 1–100 (default 50).
    #[arg(long)]
    pub limit: Option<i64>,
}

impl ListArgs {
    /// The core request these flags name.
    #[must_use]
    pub fn request(self) -> evidence_page::Request {
        evidence_page::Request {
            project_id: self.project_id,
            limit: self.limit,
            cursor: self.cursor,
            kind: self.kind,
            trust: self.trust,
            work_item_id: self.work_item_id,
        }
    }
}

/// Run one `project evidence` verb under `operator`, printing its answer as
/// JSON under `--json` and as the TTY view otherwise.
pub fn run(ctx: &mut Ctx<'_>, operator: &Envelope, json_flag: bool, args: Args) -> Result<()> {
    match args.cmd {
        EvidenceCmd::Add(a) => {
            let resp = authority::run(ctx, operator, a.request()?)?;
            project_transfer::warn_local_only(&resp.warnings)?;
            emit(json_flag, &resp, |out| render_one(out, &resp.evidence))
        }
        EvidenceCmd::List(l) => {
            let resp = authority::run(ctx, operator, l.request())?;
            emit(json_flag, &resp, |out| render_page(out, &resp))
        }
    }
}

/// One evidence record, one field per line.
pub fn render_one(out: &mut dyn std::io::Write, e: &EvidenceView) -> std::io::Result<()> {
    let dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".to_string());
    writeln!(out, "id            {}", e.id)?;
    writeln!(out, "kind          {}", e.kind)?;
    writeln!(out, "trust         {}", e.trust)?;
    writeln!(out, "work item     {}", dash(&e.work_item_id))?;
    writeln!(out, "source        {}", e.source)?;
    writeln!(out, "external id   {}", dash(&e.external_id))?;
    writeln!(out, "url           {}", dash(&e.url))?;
    writeln!(
        out,
        "creator       {}:{}",
        e.creator_principal_type, e.creator_principal_id
    )?;
    writeln!(out, "created       {}", e.created_at)
}

/// One line per record — `<createdAt>  <trust> <kind> <id>  <work item or
/// project>  <source>` — or `no evidence`, then the next page's cursor when
/// there is one.
pub fn render_page(
    out: &mut dyn std::io::Write,
    page: &evidence_page::Response,
) -> std::io::Result<()> {
    if page.evidence.is_empty() {
        writeln!(out, "no evidence")?;
    }
    for e in &page.evidence {
        let (at, trust, kind) = (&e.created_at, &e.trust, &e.kind);
        let (id, source) = (&e.id, &e.source);
        let scope = e.work_item_id.as_deref().unwrap_or("project");
        writeln!(out, "{at}  {trust:<13} {kind:<12} {id}  {scope}  {source}")?;
    }
    if let Some(cursor) = &page.next_cursor {
        writeln!(out, "next page: --cursor {cursor}")?;
    }
    Ok(())
}
