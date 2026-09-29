//! `comemory project activity <ID>` (#331): the flags and the TTY view of
//! one project's activity page, a thin shell over
//! `domains::projects::activity_page` that `project.rs` runs under the local
//! operator's envelope. Clap argument ids equal the core request's serde
//! names, so the MCP parity probe maps each flag onto `project_show`'s
//! activity view.

use clap::Args as ClapArgs;

use crate::domains::projects::activity_page::{Request, Response};

/// Args for `project activity`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// The project's UUID.
    pub id: String,
    /// `desc` (newest first, the default) or `asc` (oldest first).
    #[arg(long)]
    pub order: Option<String>,
    /// The previous page's `nextCursor`.
    #[arg(long)]
    pub cursor: Option<String>,
    /// Page size, 1–200 (default 50).
    #[arg(long)]
    pub limit: Option<i64>,
}

impl Args {
    /// The core request these flags name.
    #[must_use]
    pub fn request(self) -> Request {
        Request {
            id: self.id,
            limit: self.limit,
            cursor: self.cursor,
            order: self.order,
        }
    }
}

/// One line per event — `<createdAt>  <eventType> <entityType> <entityId>
/// <actorType>:<actorId>` — or `no activity`, then the next page's cursor
/// when there is one.
pub fn render(out: &mut dyn std::io::Write, page: &Response) -> std::io::Result<()> {
    if page.events.is_empty() {
        writeln!(out, "no activity")?;
    }
    for e in &page.events {
        let (at, event, entity) = (&e.created_at, &e.event_type, &e.entity_type);
        writeln!(
            out,
            "{at}  {event:<24} {entity} {}  {}:{}",
            e.entity_id, e.actor_principal_type, e.actor_principal_id
        )?;
    }
    if let Some(cursor) = &page.next_cursor {
        writeln!(out, "next page: --cursor {cursor}")?;
    }
    Ok(())
}
