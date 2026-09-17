//! Security primitives for `comemory serve`: a per-session bearer token and a
//! loopback Host-header guard (DNS-rebinding defense).
//!
//! The threat model is a single local user on `127.0.0.1`. The token blocks a
//! malicious web page (which, under default-deny CORS, cannot read responses
//! but could still issue requests) and DNS-rebinding from reaching the file
//! API; the Host guard rejects requests whose `Host` resolves a rebinding
//! attacker's domain. The canonicalize-and-contain path check that gates every
//! file read and write is transport-neutral and lives in
//! [`crate::utilities::path_containment`].

use crate::prelude::*;

/// Number of random bytes behind the session token (256 bits of entropy,
/// rendered as 64 lowercase-hex chars).
const TOKEN_BYTES: usize = 32;

/// Generate a fresh per-session token by reading [`TOKEN_BYTES`] from
/// `/dev/urandom` (present on every cargo-dist target — all unix) and
/// hex-encoding them. Returns an error rather than falling back to weak
/// entropy: a server that cannot authenticate must not start.
pub fn generate_token() -> Result<String> {
    random_hex(TOKEN_BYTES)
}

/// Read `bytes` bytes from `/dev/urandom` and hex-encode them (lowercase,
/// `2 * bytes` chars). The one entropy source the server uses: the session
/// token ([`generate_token`], 32 bytes) and job ids
/// (`serve::jobs::registry`, 8 bytes) are the same draw at different widths,
/// so neither can silently fall back to weaker randomness. An unreadable
/// `/dev/urandom` is an error, never a degraded default.
///
/// Delegates to [`crate::store::random_id::random_hex`] — the neutral home
/// shared with `api::gc`'s `gc_runs` row ids, so `api::` never has to depend
/// on `serve::` for the same primitive (Binding Rule 1).
pub fn random_hex(bytes: usize) -> Result<String> {
    crate::store::random_id::random_hex(bytes)
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

#[cfg(test)]
#[path = "tests/security.rs"]
mod tests;
