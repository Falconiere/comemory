//! Erase one memory: its markdown (live and `.trash/`), every mirror row, the
//! bytes of every payload its feed rows name, of the verdicts on it and of
//! the shared runs naming it — after journalling a tombstone when it is live,
//! since memory deletion is workspace-wide.

use std::path::{Path, PathBuf};

use super::{Applied, clear_copies, entries, withdraw};
use crate::domains::memories::frontmatter::Frontmatter;
use crate::domains::memories::journal;
use crate::domains::memories::replica_payload::MEMORY_ENTITY_KIND;
use crate::domains::memories::save_lock::SaveGuard;
use crate::domains::memories::trash::trash_entry_id;
use crate::prelude::*;
use crate::store::erase_rows::{self, FtsIndex};
use crate::store::replica_journal::ReplicaOrigin;
use crate::store::replica_redaction::{self, Reach};
use crate::store::{Connection, replica_read};

/// The markdown files of one memory, found where they lie.
struct Markdown {
    /// `memories/{id}-*.md`.
    live: Vec<PathBuf>,
    /// `memories/.trash/{id}-*.md`.
    trashed: Vec<PathBuf>,
}

/// Erase memory `id`. The markdown goes before the commit, under the caller's
/// guard: the source of truth must not outlive the rows, and a failure after
/// it leaves rows a rerun erases. `NotFound`, with nothing touched, when no
/// file, row or journal position names `id`.
pub(super) fn erase(
    _guard: &SaveGuard,
    memories_dir: &Path,
    conn: &mut Connection,
    id: &str,
    at: &str,
) -> Result<Applied> {
    let markdown = Markdown {
        live: files_of(memories_dir, id)?,
        trashed: files_of(&memories_dir.join(".trash"), id)?,
    };
    // Read under the write lock, so no import lands between what was found
    // and what is deleted; dropping `tx` on `NotFound` rolls it back.
    let tx = crate::store::connection::write_transaction(conn)?;
    let row = erase_rows::memory_state(&tx, id)?;
    let journalled = replica_read::revision(&tx, MEMORY_ENTITY_KIND, id)?.is_some();
    if markdown.live.is_empty() && markdown.trashed.is_empty() && row.is_none() && !journalled {
        return Err(Error::NotFound(id.to_string()));
    }
    let tombstone = live_tombstone(&markdown, row)?;
    for path in markdown.live.iter().chain(&markdown.trashed) {
        std::fs::remove_file(path)?;
    }
    if let Some((content_hash, repo)) = &tombstone {
        let repository = (!repo.is_empty()).then_some(repo.as_str());
        journal::record_tombstone(
            &tx,
            id,
            content_hash,
            repository,
            at,
            ReplicaOrigin::Local,
            None,
        )?;
    }
    let operations_withdrawn = withdraw(&tx, MEMORY_ENTITY_KIND, id, at)?;
    let mut digests = replica_redaction::redact(&tx, Reach::Entity(MEMORY_ENTITY_KIND, id), at)?;
    digests.extend(replica_redaction::redact(&tx, Reach::RunsNaming(id), at)?);
    digests.extend(erase_rows::erase_memory(&tx, id, at)?);
    let (replay_blanked, staged_removed) = clear_copies(&tx, &digests)?;
    tx.commit()?;
    Ok(Applied {
        tombstoned: tombstone.is_some(),
        digests,
        operations_withdrawn,
        staged_removed,
        replay_blanked,
        touched: vec![FtsIndex::Memories, FtsIndex::MemorySubstring],
    })
}

/// `(content_hash, repo)` the tombstone carries when the memory is live —
/// from the mirror row, or from the live file's frontmatter when the mirror
/// has none. `None` for a trashed memory, whose deletion is journalled
/// already.
fn live_tombstone(
    markdown: &Markdown,
    row: Option<erase_rows::MemoryState>,
) -> Result<Option<(String, String)>> {
    if let Some(row) = row.filter(|r| r.live) {
        return Ok(Some((row.content_hash, row.repo)));
    }
    let Some(path) = markdown.live.first() else {
        return Ok(None);
    };
    let (frontmatter, _body) = Frontmatter::split(&std::fs::read_to_string(path)?)?;
    Ok(Some((frontmatter.content_hash, frontmatter.repo)))
}

/// Every `{id}-*.md` directly under `dir`; empty when `dir` does not exist.
fn files_of(dir: &Path, id: &str) -> Result<Vec<PathBuf>> {
    Ok(entries(dir)?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| trash_entry_id(&name.to_string_lossy()).map(str::to_owned))
                .is_some_and(|found| found == id)
        })
        .collect())
}
