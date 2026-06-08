//! 64-bit SimHash over a token iterator.
//!
//! Tokens are individually hashed with siphash24; the 64 bit columns
//! are summed (1/-1) and the sign at each bit gives the SimHash.

use siphasher::sip::SipHasher24;
use std::hash::{Hash, Hasher};

/// Compute the 64-bit SimHash of an iterator of tokens.
pub fn simhash64<I, T>(tokens: I) -> u64
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let mut columns = [0i32; 64];
    for token in tokens {
        let mut h = SipHasher24::new();
        token.as_ref().hash(&mut h);
        let bits = h.finish();
        for (i, col) in columns.iter_mut().enumerate() {
            if (bits >> i) & 1 == 1 {
                *col += 1;
            } else {
                *col -= 1;
            }
        }
    }
    let mut out: u64 = 0;
    for (i, col) in columns.iter().enumerate() {
        if *col > 0 {
            out |= 1 << i;
        }
    }
    out
}

/// Hamming distance between two 64-bit values.
pub fn hamming64(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Tokenize a snippet for SimHash input: lowercase alphanumeric runs.
pub fn tokens(snippet: &str) -> Vec<String> {
    snippet
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect()
}
