//! Markdown-as-source-of-truth memory store: atomic save, load, list, delete.

use std::fs;
use std::path::PathBuf;

use time::OffsetDateTime;

use crate::config::paths::Paths;
use crate::memory::frontmatter::{Frontmatter, Kind, References, Relations};
use crate::memory::id::{memory_id, sha256_hex};
use crate::memory::slug::slug_from_body;
use crate::prelude::*;

/// One memory loaded from disk: parsed frontmatter, body string, and the path
/// it lives at on disk.
#[derive(Debug, Clone)]
pub struct MemoryRecord {
    pub frontmatter: Frontmatter,
    pub body: String,
    pub path: PathBuf,
}

/// Filesystem-backed CRUD over `memories/{id}-{slug}.md`. Stateless beyond the
/// path roots in `paths`; cheap to clone.
#[derive(Debug, Clone)]
pub struct MemoryStore {
    paths: Paths,
}

impl MemoryStore {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    /// Save a memory atomically: write to `.{id}.tmp`, then rename to
    /// `{id}-{slug}.md`. On any failure between staging and rename, the tmp
    /// file is removed so no orphaned `.tmp` files are left behind (both
    /// `fs::write` and `fs::rename` failure paths trigger cleanup).
    pub fn save(
        &self,
        body: &str,
        kind: Kind,
        repo: &str,
        tags: &[String],
        author: &str,
        quality: u8,
    ) -> Result<MemoryRecord> {
        let id = memory_id(body);
        let slug = slug_from_body(body);
        let final_path = self.paths.memories_dir().join(format!("{id}-{slug}.md"));
        let tmp_path = self.paths.memories_dir().join(format!(".{id}.tmp"));

        let content_hash = sha256_hex(body.trim_end().as_bytes());
        let fm = Frontmatter {
            id: id.clone(),
            kind,
            repo: repo.to_string(),
            tags: tags.to_vec(),
            author: author.to_string(),
            created: OffsetDateTime::now_utc(),
            quality,
            schema: 1,
            content_hash,
            references: References::default(),
            relations: Relations::default(),
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

        Ok(MemoryRecord {
            frontmatter: fm,
            body: body.trim_end().to_string(),
            path: final_path,
        })
    }

    /// Load a memory by id. Returns `Error::Other` when no file matches.
    pub fn load(&self, id: &str) -> Result<MemoryRecord> {
        let path = self.find_by_id(id)?;
        let raw = fs::read_to_string(&path)?;
        let (fm, body) = Frontmatter::split(&raw)?;
        Ok(MemoryRecord {
            frontmatter: fm,
            body,
            path,
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
        Ok(rec)
    }

    /// Enumerate every saved memory under `memories/`. Skips hidden files
    /// (e.g. `.{id}.tmp`) and the `.trash/` directory.
    pub fn list(&self) -> Result<Vec<MemoryRecord>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(self.paths.memories_dir())? {
            let entry = entry?;
            let name = entry.file_name().into_string().unwrap_or_default();
            if !name.ends_with(".md") || name.starts_with('.') {
                continue;
            }
            let raw = fs::read_to_string(entry.path())?;
            let (fm, body) = Frontmatter::split(&raw)?;
            out.push(MemoryRecord {
                frontmatter: fm,
                body,
                path: entry.path(),
            });
        }
        Ok(out)
    }

    fn find_by_id(&self, id: &str) -> Result<PathBuf> {
        let prefix = format!("{id}-");
        for entry in fs::read_dir(self.paths.memories_dir())? {
            let entry = entry?;
            let name = entry.file_name().into_string().unwrap_or_default();
            if name.starts_with(&prefix) && name.ends_with(".md") {
                return Ok(entry.path());
            }
        }
        Err(Error::Other(format!("memory not found: {id}")))
    }
}
