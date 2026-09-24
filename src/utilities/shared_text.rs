//! Free text bound for another machine (#254): a shared activity event's
//! `query` and either event kind's `actor` label.
//!
//! One policy, applied in one order. Machine paths are stripped first, because
//! they say where something lives on the machine that recorded it and nothing
//! a peer can use. The curated secret scan then runs over what would actually
//! leave; one match withholds the whole field — a query is shared entirely or
//! not at all, as a document revision is. The value is never logged either
//! way: a withheld field would otherwise leave through the log instead.

use crate::utilities::secret_scan;

/// What may leave for one free-text field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shared {
    /// The text as a peer receives it, machine paths replaced by [`PATH`].
    Kept(String),
    /// The text matched a secret rule and must not leave at all.
    Withheld,
}

/// What an absolute machine path is replaced with.
pub const PATH: &str = "<path>";

/// Apply the free-text policy to `raw`.
#[must_use]
pub fn for_share(raw: &str) -> Shared {
    let stripped = strip_machine_paths(raw);
    if secret_scan::scan(&stripped).is_some() {
        return Shared::Withheld;
    }
    Shared::Kept(stripped)
}

/// [`for_share`] for a caller label: the kept text, or `None` when the label
/// carries a secret — a label is dropped rather than marked.
#[must_use]
pub fn label_for_share(raw: &str) -> Option<String> {
    match for_share(raw) {
        Shared::Kept(text) => Some(text),
        Shared::Withheld => None,
    }
}

/// Replace every whitespace-separated token that is an absolute machine path
/// with [`PATH`], keeping the punctuation around it. Tokens are rejoined with
/// single spaces.
fn strip_machine_paths(raw: &str) -> String {
    raw.split_whitespace()
        .map(|token| {
            let core = token.trim_matches(|c: char| WRAPPERS.contains(c));
            if is_machine_path(core) {
                token.replacen(core, PATH, 1)
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Punctuation that may wrap a path inside prose.
const WRAPPERS: &str = "\"'`()[]{}<>,;:";

/// Whether `token` names a location on this machine's disk: a rooted path of
/// at least two segments, a home-relative path, a drive-letter path, or a UNC
/// share. A repository-relative path (`src/lib.rs`) or a bare `/` is not.
fn is_machine_path(token: &str) -> bool {
    if token.starts_with("~/") || token.starts_with("\\\\") {
        return true;
    }
    let bytes = token.as_bytes();
    if bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        return true;
    }
    token.starts_with('/') && token.split('/').filter(|s| !s.is_empty()).count() >= 2
}

#[cfg(test)]
#[path = "tests/shared_text.rs"]
mod tests;
