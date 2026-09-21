//! Canonical JSON bytes and their digest — the encoding a replicated payload
//! is hashed and stored under.
//!
//! `serde_json` preserves a `Map`'s insertion order, so the same payload built
//! by two code paths can serialize to two byte strings and two digests. Keys
//! are emitted sorted at every depth to remove that freedom. What a payload
//! contains stays with the domain that owns the entity.

use serde_json::{Map, Value};

use crate::prelude::*;
use crate::utilities::digest::sha256_hex;

/// Serialize `value` with every object key sorted, recursively.
///
/// # Errors
/// Propagates a `serde_json` failure — a `f64` that is NaN or infinite has no
/// JSON representation, and a payload carrying one must be refused rather
/// than silently reshaped.
pub fn to_bytes(value: &Value) -> Result<Vec<u8>> {
    let sorted = sort_value(value);
    serde_json::to_vec(&sorted).map_err(|e| Error::Other(format!("canonical json: {e}")))
}

/// Canonical bytes of `value` plus their 64-hex SHA-256 digest.
///
/// # Errors
/// Propagates [`to_bytes`].
pub fn bytes_and_digest(value: &Value) -> Result<(Vec<u8>, String)> {
    let bytes = to_bytes(value)?;
    let digest = sha256_hex(&bytes);
    Ok((bytes, digest))
}

/// The 64-hex SHA-256 digest of already-canonical `bytes`.
///
/// Used on the receiving side, which hashes what actually arrived rather than
/// re-serializing a decoded value — a payload whose bytes disagree with its
/// declared digest must be refused, not silently re-canonicalized.
#[must_use]
pub fn digest_of(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

/// Rebuild `value` with sorted object keys at every depth.
fn sort_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = Map::with_capacity(map.len());
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(child) = map.get(key) {
                    sorted.insert(key.clone(), sort_value(child));
                }
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(sort_value).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
#[path = "tests/canonical_json.rs"]
mod tests;
