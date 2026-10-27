//! Directory-tree mirroring for file-based host installs (`cursor`, `hermes`).
//!
//! The bundle's own [`extract`](super::bundle::extract) owns the same rules
//! for the shared bundle; this module is the small generic half file-based
//! hosts reuse: compare-then-copy with refusal on foreign edits, atomic
//! writes, and no deletion of files the source tree does not contain.

use std::path::{Path, PathBuf};

use crate::prelude::*;

/// Mirror the `src` tree into `dst`: copy what is missing, keep what is
/// byte-identical, and refuse — leaving `dst` untouched — when an existing
/// file differs or any path in play is a symlink.
///
/// # Errors
/// [`Error::Usage`] for a symlinked destination or differing content;
/// otherwise an [`Error::Io`] from the copy itself.
pub(super) fn mirror_dir(src: &Path, dst: &Path) -> Result<()> {
    if dst
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err(Error::Usage(format!(
            "refusing symlink install target: {}",
            dst.display()
        )));
    }
    for relative in list_files(src)? {
        let from = src.join(&relative);
        let to = dst.join(&relative);
        if to
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err(Error::Usage(format!(
                "refusing symlink install target: {}",
                to.display()
            )));
        }
        match std::fs::read(&to) {
            Ok(current) if current == std::fs::read(&from)? => {}
            Ok(_) => {
                return Err(Error::Usage(format!(
                    "{} differs from this binary; preserve or move it before reinstalling",
                    to.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let body = std::fs::read(&from)?;
                write_executable_aware(&to, &body)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Relative file paths under `dir`, sorted, so mirroring is deterministic.
fn list_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in std::fs::read_dir(&current)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.is_file() {
                files.push(
                    path.strip_prefix(dir)
                        .map(Path::to_path_buf)
                        .map_err(|error| {
                            Error::Usage(format!("install tree escaped its root: {error}"))
                        })?,
                );
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Write `body` to `path`, preserving the bundle's executable-bit rule for
/// shell scripts.
fn write_executable_aware(path: &Path, body: &[u8]) -> Result<()> {
    std::fs::write(path, body)?;
    #[cfg(unix)]
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("sh"))
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/dirs.rs"]
mod tests;
