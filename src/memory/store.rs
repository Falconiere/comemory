//! Markdown-as-source-of-truth memory store: atomic save, load, list, delete.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use time::OffsetDateTime;

use crate::config::paths::Paths;
use crate::memory::frontmatter::{Frontmatter, Kind, References, Relations};
use crate::memory::id::{memory_id, sha256_hex};
use crate::memory::slug::slug_from_body;
use crate::prelude::*;

/// Caller-supplied inputs for [`MemoryStore::save`]. Grouped into a struct
/// (rather than a growing positional list) so new frontmatter knobs extend
/// one type instead of every call site, and the argument count stays within
/// clippy's `too_many_arguments` budget.
#[derive(Debug, Clone)]
pub struct SaveParams<'a> {
    /// Memory body (markdown). Trailing whitespace is trimmed before hashing.
    pub body: &'a str,
    /// Memory taxonomy kind.
    pub kind: Kind,
    /// Repo the memory belongs to. May be empty.
    pub repo: &'a str,
    /// Tag list (already de-duplicated by the caller).
    pub tags: &'a [String],
    /// Author identifier. May be empty.
    pub author: &'a str,
    /// Quality rating 1..=5.
    pub quality: u8,
    /// Cross-memory relations written verbatim into the frontmatter
    /// (`supersedes` / `conflicts_with` / `derived_from`); materialized as
    /// `edges` rows by `store::memory_row::insert`.
    pub relations: Relations,
    /// Version-anchored code references (`--ref-file` / `--ref-symbol`)
    /// written into the frontmatter and materialized to `edges` + `code_ref`
    /// rows by `store::memory_row::insert`.
    pub references: References,
}

impl<'a> SaveParams<'a> {
    /// Minimal params: `body` + `kind` with empty repo/tags/author, default
    /// quality 3, and no relations. Test fixtures and simple callers extend
    /// via struct update syntax.
    pub fn new(body: &'a str, kind: Kind) -> Self {
        Self {
            body,
            kind,
            repo: "",
            tags: &[],
            author: "",
            quality: 3,
            relations: Relations::default(),
            references: References::default(),
        }
    }
}

/// One memory loaded from disk: parsed frontmatter, body string, the path
/// it lives at on disk, and the slug derived from the body.
#[derive(Debug, Clone)]
pub struct MemoryRecord {
    /// Parsed YAML frontmatter block.
    pub frontmatter: Frontmatter,
    /// Markdown body (frontmatter stripped).
    pub body: String,
    /// Absolute path to the memory file on disk.
    pub path: PathBuf,
    /// Filename-safe slug derived from the body's first non-empty line.
    /// Cached on the record so callers (notably the SQLite mirror in
    /// `cli::save`) don't recompute it from `body` after `save` already
    /// did the work.
    pub slug: String,
}

/// Filesystem-backed CRUD over `memories/{id}-{slug}.md`. Cheap to clone:
/// only `Paths` plus a small per-instance id→path cache populated on the
/// fly by `load` and `delete`. The cache is per-clone (not shared) — each
/// clone re-warms on first lookup, so cloning never leaks stale entries.
#[derive(Debug)]
pub struct MemoryStore {
    paths: Paths,
    /// Memoised mapping from `frontmatter.id` to the on-disk path. Populated
    /// lazily by `find_by_id` on its first cache miss; entries persist for
    /// the lifetime of this `MemoryStore`. `save` does *not* update the
    /// cache (interior mutability would force a heavier API change for
    /// little benefit) — the next `load` for that id walks `read_dir`
    /// once and warms the cache. `delete` evicts the id when it moves the
    /// file into `.trash/`.
    id_to_path: RefCell<HashMap<String, PathBuf>>,
}

impl Clone for MemoryStore {
    fn clone(&self) -> Self {
        // Don't carry the cache across clones — each clone re-warms on
        // demand so concurrent users never see a stale entry recorded by a
        // sibling clone.
        Self {
            paths: self.paths.clone(),
            id_to_path: RefCell::new(HashMap::new()),
        }
    }
}

impl MemoryStore {
    /// Construct a fresh store rooted at `paths`. The id→path cache starts
    /// empty and is populated lazily by `find_by_id`.
    pub fn new(paths: Paths) -> Self {
        Self {
            paths,
            id_to_path: RefCell::new(HashMap::new()),
        }
    }

    /// Save a memory atomically: write to `.{id}.tmp`, then rename to
    /// `{id}-{slug}.md`. On any failure between staging and rename, the tmp
    /// file is removed so no orphaned `.tmp` files are left behind (both
    /// `fs::write` and `fs::rename` failure paths trigger cleanup).
    pub fn save(&self, p: SaveParams<'_>) -> Result<MemoryRecord> {
        let body = p.body;
        let id = memory_id(body);
        let slug = slug_from_body(body);
        let final_path = self.paths.memories_dir().join(format!("{id}-{slug}.md"));
        let tmp_path = self.paths.memories_dir().join(format!(".{id}.tmp"));

        let content_hash = sha256_hex(body.trim_end().as_bytes());
        let fm = Frontmatter {
            id: id.clone(),
            kind: p.kind,
            repo: p.repo.to_string(),
            tags: p.tags.to_vec(),
            author: p.author.to_string(),
            created: OffsetDateTime::now_utc(),
            quality: p.quality,
            schema: 1,
            content_hash,
            references: p.references,
            relations: p.relations,
        };

        let rendered = fm.render(body.trim_end())?;
        write_atomic(&tmp_path, &final_path, &rendered)?;
        // A re-save of a deleted body brings its id back to life: the stale
        // `.trash/` copy would otherwise keep shadowing it in the trash
        // listing (and gc accounting) even though the memory is live again.
        self.purge_trash_copy(&id);

        // Warm the cache so a follow-up `load` for the same id hits without
        // a `read_dir` scan.
        self.id_to_path.borrow_mut().insert(id, final_path.clone());

        Ok(MemoryRecord {
            frontmatter: fm,
            body: body.trim_end().to_string(),
            path: final_path,
            slug,
        })
    }

    /// Load a memory by id. Returns `Error::NotFound` when no file matches.
    pub fn load(&self, id: &str) -> Result<MemoryRecord> {
        let path = self.find_by_id(id)?;
        let raw = fs::read_to_string(&path)?;
        let (fm, body) = Frontmatter::split(&raw)?;
        let slug = slug_from_body(&body);
        Ok(MemoryRecord {
            frontmatter: fm,
            body,
            path,
            slug,
        })
    }

    /// Rewrite an already-loaded record in place: re-render its frontmatter +
    /// body and atomically replace the file at `record.path` (stage to
    /// `.{id}.tmp`, then rename), exactly as [`MemoryStore::save`] writes a
    /// new one.
    ///
    /// The filename is `{id}-{slug}.md` and both halves derive from the body,
    /// so a metadata-only edit (`PATCH /api/v1/memories/{id}`, the
    /// reference-refresh re-pin) keeps the same path — this never renames and
    /// never re-derives the id. A caller changing the *body* must go through
    /// `save` instead, which mints the new content-derived id.
    pub fn rewrite(&self, record: &MemoryRecord) -> Result<()> {
        let tmp_path = self
            .paths
            .memories_dir()
            .join(format!(".{}.tmp", record.frontmatter.id));
        let rendered = record.frontmatter.render(record.body.trim_end())?;
        write_atomic(&tmp_path, &record.path, &rendered)
    }

    /// Bring a soft-deleted memory back: move `.trash/{id}-{slug}.md` back
    /// into `memories/` and return the record parsed from the restored file.
    /// The exact reverse of [`MemoryStore::delete`]'s file move; the SQLite
    /// mirror is the caller's half (`api::restore`).
    ///
    /// `Error::BadRequest` when `id` names a live memory — checked BEFORE the
    /// trash is consulted, so a stale trash copy can never be renamed over a
    /// live file (see [`MemoryStore::find_in_trash`]) — and `Error::NotFound`
    /// when it is in neither place.
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
    fn find_in_trash(&self, id: &str) -> Result<PathBuf> {
        if self.find_by_id(id).is_ok_and(|live| live.exists()) {
            return Err(Error::BadRequest(format!(
                "memory {id} is live, not in the trash"
            )));
        }
        self.trash_entry(id)
            .ok_or_else(|| Error::NotFound(id.to_string()))
    }

    /// `.trash/{id}-*.md` when a trashed copy of `id` exists. Shared by the
    /// restore lookup and the save-time purge so both agree on what a
    /// trashed copy is.
    fn trash_entry(&self, id: &str) -> Option<PathBuf> {
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
    fn purge_trash_copy(&self, id: &str) {
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

    /// Enumerate every saved memory under `memories/`. Skips hidden files
    /// (e.g. `.{id}.tmp`) and the `.trash/` directory. A single unreadable or
    /// malformed `.md` file is logged and skipped rather than aborting the
    /// whole listing. Results are sorted by `frontmatter.created` descending,
    /// with `frontmatter.id` ascending as a tie-breaker, so output is
    /// deterministic regardless of filesystem iteration order.
    pub fn list(&self) -> Result<Vec<MemoryRecord>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(self.paths.memories_dir())? {
            let entry = entry?;
            let name = entry.file_name().into_string().unwrap_or_default();
            let is_md = std::path::Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
            if !is_md || name.starts_with('.') {
                continue;
            }
            let raw = match fs::read_to_string(entry.path()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("memory load skipped: {} ({})", entry.path().display(), e);
                    continue;
                }
            };
            match Frontmatter::split(&raw) {
                Ok((fm, body)) => {
                    let slug = slug_from_body(&body);
                    out.push(MemoryRecord {
                        frontmatter: fm,
                        body,
                        path: entry.path(),
                        slug,
                    });
                }
                Err(e) => {
                    tracing::warn!("memory parse skipped: {} ({})", entry.path().display(), e);
                }
            }
        }
        out.sort_by(|a, b| {
            b.frontmatter
                .created
                .cmp(&a.frontmatter.created)
                .then_with(|| a.frontmatter.id.cmp(&b.frontmatter.id))
        });
        Ok(out)
    }

    /// Look up the on-disk path for `id`. Cache-first: hits return without
    /// touching the filesystem; misses fall back to a `read_dir` scan and
    /// insert the resolved entry so subsequent lookups are O(1).
    fn find_by_id(&self, id: &str) -> Result<PathBuf> {
        if let Some(p) = self.id_to_path.borrow().get(id) {
            // Cache hit. We don't re-validate that the file still exists on
            // disk — `delete` evicts entries and `load`'s subsequent
            // `read_to_string` surfaces any external removal as `io::Error`.
            return Ok(p.clone());
        }
        let prefix = format!("{id}-");
        for entry in fs::read_dir(self.paths.memories_dir())? {
            let entry = entry?;
            let name = entry.file_name().into_string().unwrap_or_default();
            if matches_prefix(&name, &prefix) {
                let path = entry.path();
                self.id_to_path
                    .borrow_mut()
                    .insert(id.to_string(), path.clone());
                return Ok(path);
            }
        }
        Err(Error::NotFound(id.to_string()))
    }
}

/// Whether directory entry `name` is the markdown file behind a memory id,
/// given that id's `{id}-` filename prefix. Shared by the live lookup
/// ([`MemoryStore::find_by_id`]) and the trash lookup
/// ([`MemoryStore::find_in_trash`]) so the two cannot disagree on what counts
/// as a memory file.
fn matches_prefix(name: &str, prefix: &str) -> bool {
    let is_md = Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
    is_md && name.starts_with(prefix)
}

/// Set `path`'s mtime to now. `fs::rename` keeps the original mtime, but the
/// trash readers (`api::gc::sweep_trash`, `api::trash::days_until_gc`) treat
/// a trashed file's mtime as its deletion instant — without this stamp a
/// memory last written 45 days ago and deleted today would be reaped by the
/// next gc under a 30-day window, with no undo window at all. Best-effort:
/// the move is the delete, so a failed stamp is logged, not fatal.
fn stamp_deleted_now(path: &Path) {
    let stamped = fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(SystemTime::now()));
    if let Err(e) = stamped {
        tracing::warn!(
            path = %path.display(),
            error = %e,
            "could not stamp the trashed file's mtime; gc retention counts from its last write"
        );
    }
}

/// Stage `contents` at `tmp_path`, then `fs::rename` it onto `final_path`.
/// On either failure the staged file is removed, so no orphaned `.tmp` is
/// left behind. The one write path shared by [`MemoryStore::save`] (new file)
/// and [`MemoryStore::rewrite`] (in-place replacement).
fn write_atomic(tmp_path: &Path, final_path: &Path, contents: &str) -> Result<()> {
    if let Err(e) = fs::write(tmp_path, contents) {
        let _ = fs::remove_file(tmp_path);
        return Err(e.into());
    }
    if let Err(e) = fs::rename(tmp_path, final_path) {
        let _ = fs::remove_file(tmp_path);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/store.rs"]
mod tests;
