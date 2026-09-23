//! A document's portable name: what it is called when it leaves this machine.
//!
//! The local id is a hash of an absolute path
//! ([`super::document::fingerprint`]), so two machines holding the same file
//! agree on nothing. A shared name is the digest of the canonical repository
//! and the document's normalized repository-relative path instead — neither of
//! them a machine path — so both machines compute the same one.
//!
//! The canonical repository arrives as an argument rather than being resolved
//! here: `RepositoryPolicy` belongs to the sync capability, which this one may
//! not depend on, and keeping resolution at the caller leaves this module
//! testable against real files with no platform to talk to.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::prelude::*;

/// How many digest BYTES a shared id carries — sixteen, rendered as 32 hex
/// characters, the same width as the local `document_id` it sits beside.
const SHARED_ID_BYTES: usize = 16;

/// What one local document is called upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedName {
    /// Canonical repository, as policy resolved it.
    pub repo: String,
    /// 32 lowercase hex chars over `repo` and `path`.
    pub shared_id: String,
    /// Normalized repository-relative path, forward slashes.
    pub path: String,
}

/// The portable name of the document at `relative_path` under `source_root`,
/// inside the checkout at `repo_root`.
///
/// `repo_root` is where the REPOSITORY starts, not the source root: a source
/// may be registered at any depth, and two machines that chose different
/// depths must still agree.
///
/// # Errors
/// [`Error::BadRequest`] when the path lands outside `repo_root`, is absolute
/// or empty, or is not UTF-8 — withheld rather than shared under a guessed
/// name another machine would compute differently.
pub fn name_for(
    repo: &str,
    repo_root: &Path,
    source_root: &Path,
    relative_path: &str,
) -> Result<SharedName> {
    if repo.trim().is_empty() {
        return Err(Error::BadRequest(
            "a shared document needs a canonical repository".to_string(),
        ));
    }
    let path = normalized_path(repo_root, source_root, relative_path)?;
    Ok(SharedName {
        shared_id: shared_id(repo, &path),
        repo: repo.to_string(),
        path,
    })
}

/// The digest the two inputs earn, so any machine computes the same id.
///
/// The NUL separator is what stops `("a/b", "c")` and `("a", "b/c")` from
/// colliding — a repository label cannot contain one.
#[must_use]
pub fn shared_id(repo: &str, path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo.as_bytes());
    hasher.update([0u8]);
    hasher.update(path.as_bytes());
    let digest = hasher.finalize();
    hex_of(&digest[..SHARED_ID_BYTES])
}

/// Lowercase-hex encode `bytes`.
fn hex_of(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// `relative_path` re-expressed against the repository root, with forward
/// slashes and no traversal left in it.
///
/// Lexical, not filesystem-resolving: the file may not exist on the machine
/// reading this name back, so the answer must not depend on what is on disk.
fn normalized_path(repo_root: &Path, source_root: &Path, relative_path: &str) -> Result<String> {
    let relative = Path::new(relative_path);
    if relative.is_absolute() {
        return Err(Error::BadRequest(format!(
            "document path {relative_path} is absolute; a shared path is repository-relative"
        )));
    }
    let joined = lexical_join(source_root, relative)?;
    let root = lexical_normalize(repo_root)?;
    let inside = joined.strip_prefix(&format!("{root}/")).ok_or_else(|| {
        Error::BadRequest(format!(
            "document path {relative_path} resolves outside the repository root"
        ))
    })?;
    if inside.is_empty() {
        return Err(Error::BadRequest(
            "document path resolves to the repository root itself".to_string(),
        ));
    }
    Ok(inside.to_string())
}

/// `base` joined with `relative`, resolving `.` and `..` lexically and
/// refusing a `..` that climbs past the start.
fn lexical_join(base: &Path, relative: &Path) -> Result<String> {
    let mut parts = split_utf8(base)?;
    for component in relative.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(Error::BadRequest(
                        "document path climbs above the filesystem root".to_string(),
                    ));
                }
            }
            std::path::Component::Normal(name) => parts.push(
                name.to_str()
                    .ok_or_else(|| {
                        Error::BadRequest("document path is not valid UTF-8".to_string())
                    })?
                    .to_string(),
            ),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(Error::BadRequest(
                    "document path is absolute; a shared path is repository-relative".to_string(),
                ));
            }
        }
    }
    Ok(parts.join("/"))
}

/// `path` with its components joined by forward slashes, `.`/`..` resolved.
fn lexical_normalize(path: &Path) -> Result<String> {
    Ok(split_utf8(path)?.join("/"))
}

/// `path`'s components as UTF-8 strings, with `.` dropped and `..` resolved.
fn split_utf8(path: &Path) -> Result<Vec<String>> {
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::RootDir => parts.push(String::new()),
            std::path::Component::Prefix(prefix) => parts.push(
                prefix
                    .as_os_str()
                    .to_str()
                    .ok_or_else(|| Error::BadRequest("path is not valid UTF-8".to_string()))?
                    .to_string(),
            ),
            std::path::Component::Normal(name) => parts.push(
                name.to_str()
                    .ok_or_else(|| Error::BadRequest("path is not valid UTF-8".to_string()))?
                    .to_string(),
            ),
        }
    }
    Ok(parts)
}

#[cfg(test)]
#[path = "tests/share.rs"]
mod tests;
