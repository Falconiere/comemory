//! `GET /sync/manifest` — 256-bucket content-hash digest for verify/repair.

use sha2::{Digest, Sha256};

use crate::api::Ctx;
use crate::api::sync::ManifestResponse;
use crate::prelude::*;
use crate::store::sync_log;

const BUCKET_COUNT: usize = 256;

/// Hash the live `content_hash` set into 256 buckets (first two hex chars).
pub fn run(ctx: &mut Ctx<'_>) -> Result<ManifestResponse> {
    let conn = ctx.conn()?;
    let head_seq = sync_log::head_seq(conn)?;
    let mut buckets: Vec<Vec<String>> = vec![Vec::new(); BUCKET_COUNT];
    let mut stmt = conn.prepare(
        "SELECT content_hash FROM memories WHERE deleted_at IS NULL ORDER BY content_hash",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for row in rows {
        let hash: String = row?;
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
    Ok(ManifestResponse {
        buckets: digests,
        head_seq,
    })
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
