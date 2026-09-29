//! Writing a file only its owner can read, all at once: the bytes go to a
//! sibling temp file created with mode `0600`, which is then renamed over the
//! target, so a reader sees the old file or the whole new one and a failed
//! write leaves no partial file behind. Used for a project transfer bundle,
//! which holds a project's full content.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::prelude::*;
use crate::utilities::uuid;

/// Write `bytes` to `path` atomically with owner-only permissions,
/// replacing any file already there.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    // A random suffix, so two writers of one path — threads of one process,
    // or a process that reuses a crashed one's PID — never share a temp file.
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(format!(".tmp-{}", uuid::new_v4()?));
    let tmp = PathBuf::from(tmp_name);
    let written = write_new(&tmp, bytes).and_then(|()| Ok(fs::rename(&tmp, path)?));
    if written.is_err() {
        // Best effort: the temp file may never have been created.
        let _ = fs::remove_file(&tmp);
    }
    written?;
    sync_parent(path)
}

/// Make the rename durable across a crash by syncing the directory that now
/// names the file (a no-op where directories cannot be opened for sync).
fn sync_parent(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::File::open(parent)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Create `path` (which must not exist) with mode `0600` and fill it.
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/private_file.rs"]
mod tests;
