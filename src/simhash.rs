//! 64-bit SimHash over a token iterator.
//!
//! Tokens are individually hashed with siphash24; the 64 bit columns
//! are summed (1/-1) and the sign at each bit gives the SimHash.

use siphasher::sip::SipHasher24;
use std::hash::{Hash, Hasher};

/// Hamming radius treated as "same memory, different wording".
/// Calibrated for short memory bodies: the median one-word edit lands at
/// Hamming ≤ 8 for bodies of ≤ 12 tokens, while genuinely distinct topics
/// sit at Hamming ≥ ~11. Shared by the query-time near-duplicate collapse
/// (`retrieval::diversify`) and the save-time duplicate warning.
pub const NEAR_DUP_HAMMING: u32 = 8;

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

/// Canonical fingerprint of a memory body: [`simhash64`] over [`tokens`].
/// The single definition shared by save, rebuild, the v4 migration
/// backfill, and the save-time duplicate check — so every writer and
/// reader of `memories.simhash` agrees on the hash.
pub fn of_body(body: &str) -> u64 {
    simhash64(tokens(body))
}

/// Hamming distance between two 64-bit values.
pub fn hamming64(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Tokenize a snippet for SimHash input: lowercased, diacritic-folded
/// alphanumeric runs. Casing/folding matches the FTS5 `identifier`
/// tokenizer (`store::tokenizer::split`) so "Café" and "café" hash
/// identically; token *boundaries* remain whole alphanumeric runs (no
/// camelCase splitting — SimHash measures body similarity, not
/// identifier recall). Stored hashes were recomputed by the v5
/// migration (`migrate::rehash_simhashes`); changing this function
/// again requires another re-hash migration.
pub fn tokens(snippet: &str) -> Vec<String> {
    snippet
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| crate::store::tokenizer::split::fold_diacritics(&s.to_lowercase()))
        .filter(|s| !s.is_empty())
        .collect()
}
