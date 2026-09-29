//! Version-4 UUIDs for client-generated identities (#326): minted from the
//! shared `/dev/urandom` helper, and checked in the canonical hyphenated
//! shape the platform's `z.uuid()` and keyset cursor accept.

use crate::prelude::*;
use crate::store::random_id::random_hex;

/// A fresh random (version 4, RFC 9562 variant) UUID, lowercase and
/// hyphenated.
pub fn new_v4() -> Result<String> {
    let random = random_hex(16)?;
    // The version nibble is `4`; the variant nibble is `10xx` — one of 8, 9,
    // a or b, picked by the random nibble's low two bits.
    let nibble = u8::from_str_radix(&random[16..17], 16).unwrap_or(0);
    let variant = match nibble % 4 {
        0 => '8',
        1 => '9',
        2 => 'a',
        _ => 'b',
    };
    let hex = format!(
        "{}4{}{variant}{}",
        &random[..12],
        &random[13..16],
        &random[17..]
    );
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

/// `value` in canonical lowercase form when it is an 8-4-4-4-12 hex UUID of
/// any version, in either case; `None` otherwise.
#[must_use]
pub fn canonical(value: &str) -> Option<String> {
    let groups: Vec<&str> = value.split('-').collect();
    let shaped = groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(group, len)| group.len() == len && group.bytes().all(|b| b.is_ascii_hexdigit()));
    shaped.then(|| value.to_ascii_lowercase())
}

#[cfg(test)]
#[path = "tests/uuid.rs"]
mod tests;
