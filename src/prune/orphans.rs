//! Orphan detection: trash entries whose live counterpart is gone.
//!
//! An "orphan" here is a markdown file under `memories/.trash/` whose id
//! prefix does not appear in the live on-disk listing. In practice every
//! trashed file qualifies (soft-delete moves the live file aside), so this
//! is the input set for both reporting and `gc`-style purges.

use std::collections::HashSet;

use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::prelude::*;

/// Scan `memories/.trash/` and return the ids of entries that no longer have
/// a matching live memory on disk. Returns an empty vector when the trash
/// directory does not exist (e.g. before any soft-delete has run).
pub fn detect(paths: &Paths) -> Result<Vec<String>> {
    let on_disk: HashSet<String> = MemoryStore::new(paths.clone())
        .list()?
        .into_iter()
        .map(|m| m.frontmatter.id)
        .collect();

    let mut orphans = Vec::new();
    let rd = match std::fs::read_dir(paths.trash_dir()) {
        Ok(rd) => rd,
        Err(_) => return Ok(orphans),
    };
    for entry in rd.flatten() {
        let name = entry.file_name().into_string().unwrap_or_default();
        if !name.ends_with(".md") || name.starts_with('.') {
            continue;
        }
        if let Some(id_part) = name.split('-').next()
            && !on_disk.contains(id_part)
        {
            orphans.push(id_part.to_string());
        }
    }
    Ok(orphans)
}
