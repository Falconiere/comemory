//! File-based install into the hermes skills dir.
//!
//! Hermes exposes no plugin CLI, so install copies the bundle's skill dirs
//! into `<config>/skills/`, keeping their names so they match each skill's
//! `name:` frontmatter. Hooks and MCP have no hermes story; skills carry the
//! `comemory.sh` wrapper fallback for hosts without MCP.

use std::path::{Path, PathBuf};

use crate::prelude::*;

use super::dirs::mirror_dir;

/// Mirror every skill dir from the extracted bundle into `skills/`.
/// Idempotent like every other writer here: identical content is a no-op and
/// differing content is refused with the tree left untouched.
///
/// # Errors
/// [`Error::Usage`] for a symlinked target or differing content; otherwise
/// an [`Error::Io`] from the copy.
pub(super) fn install(config_dir: &Path, root: &Path) -> Result<()> {
    let source = root.join("plugins/comemory/skills");
    let target = config_dir.join("skills");
    let mut names: Vec<PathBuf> = std::fs::read_dir(&source)?
        .map(|entry| entry.map(|entry| PathBuf::from(entry.file_name())))
        .collect::<std::io::Result<_>>()?;
    names.sort();
    for name in names {
        let from = source.join(&name);
        if !from.is_dir() {
            continue;
        }
        mirror_dir(&from, &target.join(name))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/hermes.rs"]
mod tests;
