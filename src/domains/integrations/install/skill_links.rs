//! Skill installs for file-based hosts (`cursor-agent`, `hermes`): each
//! bundled skill dir is **linked**, not copied, into the host's skills dir.
//!
//! A link keeps the skill's wrapper scripts beside the bundle `lib/` they
//! source (`scripts/../../../lib`, which the kernel resolves physically
//! through the link), turns an upgrade into an atomic repoint at the new
//! version's bundle, and leaves no copied file to compare — the bundle
//! itself already refuses operator edits on extract.

use std::path::{Path, PathBuf};

use crate::prelude::*;

/// What already sits where a skill link belongs.
#[derive(Debug, PartialEq, Eq)]
enum Existing {
    /// Nothing: create the link.
    Missing,
    /// A link to exactly this bundle's skill: nothing to do.
    Current,
    /// A link into another comemory version's bundle: repoint it.
    Stale,
}

/// Link every skill dir under `<root>/plugins/comemory/skills` into
/// `target`. An existing entry is replaced only when it is a symlink into
/// `root`'s parent — a comemory bundle of any version; a real directory, a
/// file or a link elsewhere is refused, and every entry is checked before
/// any link is written.
///
/// # Errors
/// [`Error::Usage`] for an entry comemory did not write; otherwise an
/// [`Error::Io`] from reading the bundle or writing a link.
pub(super) fn link_skills(root: &Path, target: &Path) -> Result<()> {
    let owned = root
        .parent()
        .ok_or_else(|| Error::Usage("bundle needs a parent".into()))?;
    let source = root.join("plugins/comemory/skills");
    let mut planned = Vec::new();
    for name in skill_names(&source)? {
        let wanted = source.join(&name);
        let link = target.join(&name);
        match existing(&link, &wanted, owned)? {
            Existing::Current => {}
            Existing::Missing | Existing::Stale => planned.push((wanted, link)),
        }
    }
    if planned.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(target)?;
    for (wanted, link) in planned {
        replace_link(&wanted, &link)?;
    }
    Ok(())
}

/// The skill dir names under `source`, sorted so installs are deterministic.
/// Names come from the bundle tree so a renamed skill cannot drift out of
/// sync with a hardcoded list.
fn skill_names(source: &Path) -> Result<Vec<PathBuf>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            names.push(PathBuf::from(entry.file_name()));
        }
    }
    names.sort();
    Ok(names)
}

/// Classify what sits at `link`, refusing anything comemory did not write.
fn existing(link: &Path, wanted: &Path, owned: &Path) -> Result<Existing> {
    let meta = match link.symlink_metadata() {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Existing::Missing);
        }
        Err(error) => return Err(error.into()),
    };
    if !meta.file_type().is_symlink() {
        return Err(Error::Usage(format!(
            "{} exists and is not a comemory skill link; preserve or move it before reinstalling",
            link.display()
        )));
    }
    let points = std::fs::read_link(link)?;
    if points == wanted {
        Ok(Existing::Current)
    } else if super::canonical_or_raw(&points).starts_with(super::canonical_or_raw(owned)) {
        Ok(Existing::Stale)
    } else {
        Err(Error::Usage(format!(
            "{} links to {}, which comemory did not write; move it before reinstalling",
            link.display(),
            points.display()
        )))
    }
}

/// Point `link` at `wanted` atomically: build the link under a temporary
/// sibling name, then rename it over whatever link was there.
fn replace_link(wanted: &Path, link: &Path) -> Result<()> {
    let name = link
        .file_name()
        .ok_or_else(|| Error::Usage(format!("bad skill link path: {}", link.display())))?;
    let temp = link.with_file_name(format!(
        ".{}.comemory-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    match temp.symlink_metadata() {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::remove_file(&temp)?,
        Ok(_) => {
            return Err(Error::Usage(format!(
                "refusing to replace {}: not a link",
                temp.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    symlink_dir(wanted, &temp)?;
    std::fs::rename(&temp, link)?;
    Ok(())
}

/// Create a directory symlink. Release targets are unix-only.
#[cfg(unix)]
fn symlink_dir(wanted: &Path, link: &Path) -> Result<()> {
    std::os::unix::fs::symlink(wanted, link)?;
    Ok(())
}

/// Create a directory symlink. Release targets are unix-only.
#[cfg(not(unix))]
fn symlink_dir(_wanted: &Path, link: &Path) -> Result<()> {
    Err(Error::Unavailable(format!(
        "skill links are unix-only: {}",
        link.display()
    )))
}

#[cfg(test)]
#[path = "tests/skill_links.rs"]
mod tests;
