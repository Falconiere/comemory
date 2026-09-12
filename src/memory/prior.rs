//! `Prior` — what the markdown tree already holds for a memory id, looked up
//! by [`MemoryStore::prior`] before a save. The live `memories/{id}-*.md` is
//! consulted first, then `.trash/{id}-*.md`, the same order
//! `MemoryStore::find_in_trash` uses, so the two can never disagree on which
//! copy of an id counts. One lookup backs both `api::save`'s replay rules —
//! the 32-bit collision guard, `created` preservation, and the
//! `created: bool` outcome report — and `api::sync::import_state`'s
//! pulled-record collision rule (Binding Rule 1).
//!
//! Markdown is the source of truth, so this reads the file and never the
//! `memories` mirror row: on the documented "markdown written, mirror
//! failed, run `comemory rebuild`" failure mode the file is what exists.

use std::fs;
use std::path::Path;

use time::OffsetDateTime;

use crate::memory::frontmatter::Frontmatter;
use crate::memory::store::MemoryStore;
use crate::prelude::*;

/// The frontmatter facts an existing memory file contributes to a re-save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prior {
    /// Frontmatter `created` of the existing file — carried onto the
    /// re-save so a replay never moves a memory's creation instant forward.
    pub created: OffsetDateTime,
    /// Frontmatter `content_hash` of the existing file.
    pub content_hash: String,
    /// The file was found under `.trash/`, not `memories/`: the save
    /// revives it.
    pub trashed: bool,
}

impl Prior {
    /// Same id, different body. The id is the first 4 bytes of the very
    /// digest `content_hash` spells out in full, so equal bodies cannot
    /// disagree here — a mismatch is a 32-bit content-address collision the
    /// save must refuse rather than silently overwrite.
    pub fn collides_with(&self, content_hash: &str) -> bool {
        self.content_hash != content_hash
    }
}

impl MemoryStore {
    /// What the store already holds for `id`: the live file first, then the
    /// trash copy, `None` when neither exists. A `memories/` directory that
    /// does not exist yet (the first save ever) is `None`, not an error. A
    /// file that exists but cannot be parsed propagates its
    /// [`Frontmatter::split`] error: the save never guesses about, and
    /// never overwrites, a file it could not read.
    pub fn prior(&self, id: &str) -> Result<Option<Prior>> {
        match self.find_by_id(id) {
            Ok(path) => return read_prior(&path, false).map(Some),
            Err(Error::NotFound(_)) => {}
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        match self.trash_entry(id) {
            Some(path) => read_prior(&path, true).map(Some),
            None => Ok(None),
        }
    }
}

/// Parse the frontmatter at `path` into a [`Prior`].
fn read_prior(path: &Path, trashed: bool) -> Result<Prior> {
    let raw = fs::read_to_string(path)?;
    let (fm, _body) = Frontmatter::split(&raw)?;
    Ok(Prior {
        created: fm.created,
        content_hash: fm.content_hash,
        trashed,
    })
}

#[cfg(test)]
#[path = "tests/prior.rs"]
mod tests;
