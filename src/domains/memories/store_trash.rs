//! The trash half of [`MemoryStore`] — soft delete, restore, and the
//! lookups both need.
//!
//! Split out of `store.rs` to keep each file inside the structure gate's line
//! ceiling. It is one `impl MemoryStore` block, not a second type: the trash
//! is the same store's other half of the same markdown tree, and the
//! live-before-trash ordering in [`MemoryStore::find_in_trash`] is the rule
//! that keeps a stale trash copy from being renamed over a live file.

use std::fs;
use std::path::PathBuf;

use crate::domains::memories::frontmatter::Frontmatter;
use crate::domains::memories::slug::slug_from_body;
use crate::domains::memories::store::{
    MemoryRecord, MemoryStore, matches_prefix, stamp_deleted_now,
};
use crate::prelude::*;

impl MemoryStore {
    /// The trashed record for `id`, read where it lies.
    ///
    /// Lets a caller record what a restore is about to do — the canonical id
    /// and the file that will move — BEFORE [`restore`](Self::restore) moves
    /// it. `path` is the `.trash/` path, not the live one.
    ///
    /// # Errors
    /// Propagates the trash lookup, the read and the frontmatter parse.
    pub fn trashed_record(&self, id: &str) -> Result<MemoryRecord> {
        let trash_path = self.find_in_trash(id)?;
        let raw = fs::read_to_string(&trash_path)?;
        let (fm, body) = Frontmatter::split(&raw)?;
        let slug = slug_from_body(&body);
        Ok(MemoryRecord {
            frontmatter: fm,
            body,
            path: trash_path,
            slug,
        })
    }

    /// Bring a soft-deleted memory back: move `.trash/{id}-{slug}.md` back
    /// into `memories/` and return the record parsed from the restored file.
    ///
    /// The exact reverse of [`MemoryStore::delete`]'s file move; the SQLite
    /// mirror is the caller's half (`memories::restore`).
    ///
    /// `Error::BadRequest` when `id` names a live memory — checked BEFORE the
    /// trash is consulted, so a stale trash copy can never be renamed over a
    /// live file (see [`MemoryStore::find_in_trash`]) — and `Error::NotFound`
    /// when it is in neither place.
    ///
    /// # Errors
    /// Propagates the trash lookup, the rename, the read and the frontmatter
    /// parse.
    pub fn restore(&self, id: &str) -> Result<MemoryRecord> {
        let trash_path = self.find_in_trash(id)?;
        let file_name = trash_path
            .file_name()
            .ok_or_else(|| {
                Error::Other(format!(
                    "trashed memory path has no file name: {}",
                    trash_path.display()
                ))
            })?
            .to_owned();
        let live_path = self.paths.memories_dir().join(file_name);
        fs::rename(&trash_path, &live_path)?;
        // Warm the cache at the restored path — `delete` evicted it.
        self.id_to_path
            .borrow_mut()
            .insert(id.to_string(), live_path.clone());
        let raw = fs::read_to_string(&live_path)?;
        let (fm, body) = Frontmatter::split(&raw)?;
        let slug = slug_from_body(&body);
        Ok(MemoryRecord {
            frontmatter: fm,
            body,
            path: live_path,
            slug,
        })
    }

    /// Locate the file [`MemoryStore::restore`] should move back. The LIVE
    /// tree is checked first, and a live id is `BadRequest` even when a trash
    /// copy exists: a same-body re-save after a delete recreates
    /// `{id}-{slug}.md` under the very same name, and `fs::rename` out of
    /// `.trash/` would silently replace that live file (and its newer
    /// frontmatter) with the stale pre-delete copy. Only an id absent from
    /// both trees is `NotFound`.
    pub(super) fn find_in_trash(&self, id: &str) -> Result<PathBuf> {
        if self.find_by_id(id).is_ok_and(|live| live.exists()) {
            return Err(Error::BadRequest(format!(
                "memory {id} is live, not in the trash"
            )));
        }
        self.trash_entry(id)
            .ok_or_else(|| Error::NotFound(id.to_string()))
    }

    /// `.trash/{id}-*.md` when a trashed copy of `id` exists. Shared by the
    /// restore lookup, the save-time purge and [`MemoryStore::prior`] (hence
    /// `pub(crate)`) so all agree on what a trashed copy is.
    pub(crate) fn trash_entry(&self, id: &str) -> Option<PathBuf> {
        let prefix = format!("{id}-");
        fs::read_dir(self.paths.trash_dir())
            .ok()?
            .flatten()
            .find(|entry| matches_prefix(&entry.file_name().to_string_lossy(), &prefix))
            .map(|entry| entry.path())
    }

    /// Remove a leftover `.trash/` copy of `id` once the id is live again
    /// (a re-save of a deleted body). Best-effort: the live file is already
    /// the source of truth, so a failure is logged rather than propagated.
    pub(super) fn purge_trash_copy(&self, id: &str) {
        let Some(stale) = self.trash_entry(id) else {
            return;
        };
        match fs::remove_file(&stale) {
            Ok(()) => tracing::debug!(
                path = %stale.display(),
                "removed the stale trash copy of a re-saved memory"
            ),
            Err(e) => tracing::warn!(
                path = %stale.display(),
                error = %e,
                "could not remove the stale trash copy of a re-saved memory"
            ),
        }
    }

    /// Soft-delete a memory by moving it into `memories/.trash/`. Returns the
    /// record as it existed before deletion.
    pub fn delete(&self, id: &str) -> Result<MemoryRecord> {
        let rec = self.load(id)?;
        let file_name = rec
            .path
            .file_name()
            .ok_or_else(|| {
                Error::Other(format!(
                    "memory path has no file name: {}",
                    rec.path.display()
                ))
            })?
            .to_owned();
        let trash_dir = self.paths.trash_dir();
        fs::create_dir_all(&trash_dir)?;
        let trash_path = trash_dir.join(&file_name);
        fs::rename(&rec.path, &trash_path)?;
        stamp_deleted_now(&trash_path);
        // Evict the cached entry — the file is no longer at the live path.
        self.id_to_path.borrow_mut().remove(id);
        Ok(rec)
    }
}
