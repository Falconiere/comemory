//! `GET /sync/manifest` — 256-bucket content-hash digest for verify/repair.

use sha2::{Digest, Sha256};

use crate::domains::sync::exchange::ManifestResponse;
use crate::prelude::*;
use crate::store::{sync_log, sync_manifest};
use crate::utilities::context::Ctx;

const BUCKET_COUNT: usize = 256;

/// Hash the live `content_hash` set into 256 buckets (first two hex chars).
pub fn run(ctx: &mut Ctx<'_>) -> Result<ManifestResponse> {
    let conn = ctx.conn()?;
    let head_seq = sync_log::head_seq(conn)?;
    Ok(from_hashes(
        head_seq,
        sync_manifest::live_content_hashes(conn)?,
    ))
}

/// Build the protocol's 256-bucket digest from an already authorized hash set.
#[must_use]
pub fn from_hashes(head_seq: i64, hashes: Vec<String>) -> ManifestResponse {
    let mut buckets: Vec<Vec<String>> = vec![Vec::new(); BUCKET_COUNT];
    for hash in hashes {
        if hash.len() < 2 {
            continue;
        }
        if let Some(idx) = bucket_index(&hash[..2]) {
            buckets[idx].push(hash);
        }
    }
    let digests = buckets
        .into_iter()
        .map(|mut hashes| {
            hashes.sort();
            let joined = hashes.join("");
            let digest = Sha256::digest(joined.as_bytes());
            hex_encode(&digest)
        })
        .collect();
    ManifestResponse {
        buckets: digests,
        head_seq,
    }
}

fn bucket_index(prefix: &str) -> Option<usize> {
    u8::from_str_radix(prefix, 16).ok().map(usize::from)
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
