//! Security primitives for `comemory serve`: a per-session bearer token, a
//! loopback Host-header guard (DNS-rebinding defense), and the
//! canonicalize-and-contain path check that gates every file read and write.
//!
//! The threat model is a single local user on `127.0.0.1`. The token blocks a
//! malicious web page (which, under default-deny CORS, cannot read responses
//! but could still issue requests) and DNS-rebinding from reaching the file
//! API; the Host guard rejects requests whose `Host` resolves a rebinding
//! attacker's domain; the containment check ensures a crafted `file:<repo>:…`
//! id can never escape the repo root, even through `..` or a symlink.

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

use crate::prelude::*;

/// Number of random bytes behind the session token (256 bits of entropy,
/// rendered as 64 lowercase-hex chars).
const TOKEN_BYTES: usize = 32;

/// Generate a fresh per-session token by reading [`TOKEN_BYTES`] from
/// `/dev/urandom` (present on every cargo-dist target — all unix) and
/// hex-encoding them. Returns an error rather than falling back to weak
/// entropy: a server that cannot authenticate must not start.
pub fn generate_token() -> Result<String> {
    let mut f = std::fs::File::open("/dev/urandom").map_err(Error::Io)?;
    let mut buf = [0u8; TOKEN_BYTES];
    f.read_exact(&mut buf).map_err(Error::Io)?;
    let mut hex = String::with_capacity(TOKEN_BYTES * 2);
    for b in buf {
        // Infallible: writing to a String never errors.
        let _ = write!(hex, "{b:02x}");
    }
    Ok(hex)
}

/// True when `provided` equals the session token. Absent token → false.
///
/// The comparison is constant-time over the token bytes (XOR-accumulate, no
/// early byte-wise exit) so a network attacker cannot recover the token one
/// byte at a time from response-timing differences. A length mismatch returns
/// early, but the token length is fixed and public (64 hex chars), so that
/// leaks nothing secret.
pub fn token_matches(provided: Option<&str>, expected: &str) -> bool {
    let provided = match provided {
        Some(p) => p.as_bytes(),
        None => return false,
    };
    let expected = expected.as_bytes();
    if provided.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in provided.iter().zip(expected.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// True when the `Host` header names a loopback host. Only the hostname part
/// is checked (the port is irrelevant to the rebinding defense); the bare
/// loopback literals plus `localhost` are accepted. An absent/empty Host is
/// rejected so a rebinding request without a recognizable host cannot pass.
pub fn host_is_loopback(host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    // A bare IPv6 literal carries its own colons, so it cannot be naively
    // port-stripped — match it before splitting.
    if host == "::1" {
        return true;
    }
    // Strip a trailing `:port`. IPv6 literals are bracketed (`[::1]:port`),
    // so keep everything through the closing bracket; otherwise split on the
    // last colon.
    let hostname = match host.rfind(']') {
        Some(close) => &host[..=close],
        None => host.rsplit_once(':').map_or(host, |(h, _)| h),
    };
    matches!(hostname, "127.0.0.1" | "localhost" | "[::1]")
}

/// Resolve `rel` (a repo-relative path from a `file:<repo>:<path>` id) against
/// the canonical repo `root`, guaranteeing the result stays inside `root`.
///
/// `root` MUST already be canonical (the caller canonicalizes it). The check
/// rejects absolute paths, `..` components, and NUL up front, then
/// canonicalizes the target — resolving symlinks — and asserts it is still
/// prefixed by `root`. For a not-yet-existing file (a fresh `PUT`), the parent
/// directory is canonicalized instead and the filename re-appended, so a
/// symlinked parent escaping the root is still caught.
pub fn resolve_within(root: &Path, rel: &str) -> Result<PathBuf> {
    if rel.is_empty() {
        return Err(Error::BadRequest("empty path".into()));
    }
    if rel.contains('\0') {
        return Err(Error::Forbidden("NUL in path".into()));
    }
    let rel_path = Path::new(rel);
    for comp in rel_path.components() {
        match comp {
            Component::ParentDir => {
                return Err(Error::Forbidden("'..' not allowed in path".into()));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(Error::Forbidden("absolute path not allowed".into()));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    let candidate = root.join(rel_path);
    let canonical = match candidate.canonicalize() {
        Ok(c) => c,
        Err(_) => {
            // Not-yet-existing file: canonicalize the parent, re-append name.
            let parent = candidate
                .parent()
                .ok_or_else(|| Error::BadRequest("path has no parent".into()))?;
            let name = candidate
                .file_name()
                .ok_or_else(|| Error::BadRequest("path has no file name".into()))?;
            let parent_canon = parent.canonicalize().map_err(Error::Io)?;
            parent_canon.join(name)
        }
    };
    if !canonical.starts_with(root) {
        return Err(Error::Forbidden("path escapes repo root".into()));
    }
    Ok(canonical)
}
