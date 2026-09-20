//! TTY writers for the `comemory architecture` family. JSON goes through
//! [`json::write`]; everything a human reads is built here, so the domain
//! never formats a line.

use std::io::Write as _;

use crate::cli::output::tty;
use crate::domains::architecture::check::Drift;
use crate::domains::architecture::learn::Learned;
use crate::domains::architecture::mermaid;
use crate::domains::architecture::model::Model;
use crate::domains::architecture::save::Saved;
use crate::prelude::*;

/// The component table: one line per component, then the edges.
pub fn write_model(model: &Model) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} — {} components, {} edges ({})",
        model.repo,
        model.components.len(),
        model.edges.len(),
        model.generated_at
    )?;
    for c in &model.components {
        writeln!(
            out,
            "  {}  {:<28} {:>4} files  {}",
            tty::score(c.rank),
            c.id,
            c.files,
            c.name
        )?;
    }
    for e in &model.edges {
        writeln!(
            out,
            "  {} -> {} ({}, weight {})",
            e.from,
            e.to,
            serde_json::to_string(&e.kind)?.trim_matches('"'),
            e.weight
        )?;
    }
    Ok(())
}

/// The Mermaid source, unadorned so it can be piped or pasted.
pub fn write_mermaid(model: &Model) -> Result<()> {
    let mut out = std::io::stdout().lock();
    write!(out, "{}", mermaid::render(model))?;
    Ok(())
}

/// What a save stored.
pub fn write_saved(saved: &Saved) -> Result<()> {
    let mut out = std::io::stdout().lock();
    let verb = if saved.created { "saved" } else { "unchanged" };
    writeln!(
        out,
        "{verb} {} — {} components, {} edges",
        saved.id, saved.components, saved.edges
    )?;
    if let Some(old) = &saved.superseded {
        writeln!(out, "  supersedes {old}")?;
    }
    writeln!(out, "  {}", tty::dim(&saved.path))?;
    Ok(())
}

/// The drift report, grouped by kind.
pub fn write_drift(drift: &Drift) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{} — model {}", drift.repo, drift.model_id)?;
    for s in &drift.stale_members {
        writeln!(out, "  stale     {} -> {}", s.component, s.member)?;
    }
    for u in &drift.unmapped {
        writeln!(
            out,
            "  unmapped  {} ({} files, rank {})",
            u.path,
            u.files,
            tty::score(u.rank)
        )?;
    }
    for e in &drift.missing_edges {
        writeln!(
            out,
            "  edge      {} -> {} (weight {})",
            e.from, e.to, e.weight
        )?;
    }
    writeln!(out, "drift: {}", drift.drift_count)?;
    Ok(())
}

/// What a learn run did, including the dry-run case.
pub fn write_learned(learned: &Learned) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "prompt: {}", learned.prompt_path)?;
    match (&learned.command, &learned.saved) {
        (Some(command), Some(saved)) => {
            writeln!(out, "agent:  {command}")?;
            drop(out);
            write_saved(saved)?;
        }
        _ => writeln!(out, "dry run: no agent was started")?,
    }
    Ok(())
}
