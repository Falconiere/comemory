//! `daemon.token` and the two-way proof over it.
//!
//! The token is 32 random bytes (64 hex), created once with mode 0600 and
//! never sent over the socket. Each side proves it can read the token by
//! hashing the other side's fresh nonce with it, so a foreign process at the
//! socket path — another user's, another directory's coordinator, a leftover
//! listener — fails the client's check before the client reveals anything.

use std::io::{Read as _, Write as _};
use std::path::PathBuf;

use crate::config::Paths;
use crate::prelude::*;
use crate::store::random_id::random_hex;
use crate::utilities::digest::sha256_hex;

/// Token file name, beside the data.
pub const TOKEN_FILE: &str = "daemon.token";

/// Length of a well-formed token.
const TOKEN_LEN: usize = 64;

/// Where the token lives.
#[must_use]
pub fn token_path(paths: &Paths) -> PathBuf {
    paths.data_dir().join(TOKEN_FILE)
}

/// Read the token; `Ok(None)` when no coordinator ever ran here.
///
/// # Errors
/// The file exists but cannot be read or is malformed.
pub fn load(paths: &Paths) -> Result<Option<String>> {
    let mut raw = String::new();
    match std::fs::File::open(token_path(paths)) {
        Ok(mut file) => file.read_to_string(&mut raw)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let token = raw.trim();
    if token.len() != TOKEN_LEN || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Unavailable(format!(
            "{} is malformed — remove it and run `comemory sync daemon repair`",
            token_path(paths).display()
        )));
    }
    Ok(Some(token.to_string()))
}

/// Read the token, creating it (0600) when absent. The token is written to a
/// private temporary file and published with a hard link, which fails when
/// the name exists: a reader never sees a half-written token, and racing
/// creators all end up with the one that linked first.
///
/// # Errors
/// The file cannot be created, written or read.
pub fn load_or_create(paths: &Paths) -> Result<String> {
    if let Some(token) = load(paths)? {
        return Ok(token);
    }
    let token = random_hex(TOKEN_LEN / 2)?;
    let tmp = paths.data_dir().join(format!(".daemon.token.{}", nonce()?));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    let linked = std::fs::hard_link(&tmp, token_path(paths));
    std::fs::remove_file(&tmp)?;
    match linked {
        Ok(()) => Ok(token),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => load(paths)?
            .ok_or_else(|| Error::Unavailable("daemon.token vanished while it was read".into())),
        Err(e) => Err(e.into()),
    }
}

/// A fresh nonce (16 random bytes, hex).
///
/// # Errors
/// `/dev/urandom` cannot be read.
pub fn nonce() -> Result<String> {
    random_hex(16)
}

/// Which side of the handshake a proof is for — each hashes a different
/// label into the digest, so a server's answer can never be replayed as a
/// client's request or vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The server's proof, sent over the client's nonce.
    Server,
    /// The client's proof, sent over the server's nonce.
    Client,
}

/// `side`'s proof over `nonce`, bound to `token`.
#[must_use]
pub fn proof(side: Side, nonce: &str, token: &str) -> String {
    let label = match side {
        Side::Server => "comemory-daemon-server",
        Side::Client => "comemory-daemon-client",
    };
    sha256_hex(format!("{label}\0{nonce}\0{token}").as_bytes())
}

/// Compare two proofs without an early exit on the first differing byte.
#[must_use]
pub fn matches(expected: &str, got: &str) -> bool {
    expected.len() == got.len()
        && expected
            .bytes()
            .zip(got.bytes())
            .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

#[cfg(test)]
#[path = "tests/handshake.rs"]
mod tests;
