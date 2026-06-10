//! Markdown-as-source-of-truth memory store: atomic save, load, list, delete.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

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
        }
    }
}

/// One memory loaded from disk: parsed frontmatter, body string, the path
/// it lives at on disk, and the slug derived from the body.
#[derive(Debug, Clone)]
pub struct MemoryRecord {
    pub frontmatter: Frontmatter,
    pub body: String,
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
            references: References::default(),
            relations: p.relations,
        };

        let rendered = fm.render(body.trim_end())?;
        if let Err(e) = fs::write(&tmp_path, rendered) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e.into());
        }

        if let Err(e) = fs::rename(&tmp_path, &final_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e.into());
        }

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
            if !name.ends_with(".md") || name.starts_with('.') {
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
            if name.starts_with(&prefix) && name.ends_with(".md") {
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
