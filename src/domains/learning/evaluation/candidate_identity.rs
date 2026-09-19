//! Domain-qualified candidate identity and its reference-string codec: the
//! part of the candidate observation contract that says *what* a result is.
//!
//! Identity keys are stable across re-indexing; the content version is the one
//! field that changes when the content does. Code identity is
//! `(repo, path, symbol)` — what `domains::learning::code_feedback` keys its
//! counters by — never the `code_symbols` rowid a re-index recycles. The
//! reference string is the opaque token the reranker process protocol passes
//! around. Contract: `docs/designs/2026-09-18-domain-aware-retrieval-benchmark.md`.

use serde::{Deserialize, Serialize};

use crate::prelude::*;

/// The corpus a candidate came from. The string forms are exactly the
/// `retrieval::unified::fuse_domains::DOMAIN_*` labels already carried on
/// `UnifiedHit::domain`; this contract does not mint a second vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CandidateDomain {
    /// Hand-authored markdown in `memories`.
    Memory,
    /// An extracted symbol in `code_symbols`.
    Code,
    /// An indexed external document in `documents`.
    Document,
}

impl CandidateDomain {
    /// The wire spelling: `memory`, `code` or `document`.
    pub fn as_str(self) -> &'static str {
        match self {
            CandidateDomain::Memory => "memory",
            CandidateDomain::Code => "code",
            CandidateDomain::Document => "document",
        }
    }

    /// Every domain, in the order metrics report them.
    pub fn all() -> [CandidateDomain; 3] {
        [
            CandidateDomain::Memory,
            CandidateDomain::Code,
            CandidateDomain::Document,
        ]
    }
}

/// A memory candidate: the id plus the body digest that is its content version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryIdentity {
    /// `memories.id` — the 8-hex prefix of `sha256(body.trim_end())`.
    pub memory_id: String,
    /// `sha256(body.trim_end())` of the body retrieval returned, equal by
    /// construction to `memories.content_hash` and the frontmatter field.
    pub content_hash: String,
}

/// A code candidate: the stable identity triple plus the indexed blob OID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeIdentity {
    /// `code_symbols.repo`.
    pub repo: String,
    /// `code_symbols.path`, relative to the repo root.
    pub path: String,
    /// `code_symbols.symbol`; the PARENT's name for a coalesced cAST chunk.
    pub symbol: String,
    /// `code_symbols.blob_oid` — the file's git blob OID at index time.
    pub blob_oid: String,
}

/// A document candidate: the parent document plus which passage matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentIdentity {
    /// `documents.id` — the 32-hex parent id, derived and not hand-writable.
    pub document_id: String,
    /// `source_files.relative_path` — the parent's human-writable stable name,
    /// and the key a reviewed judgment addresses a document by. Part of the
    /// identity precisely so a judgment never has to match on a locator.
    pub path: String,
    /// `documents.revision_hash` — the parent's content version.
    pub revision_hash: String,
    /// `document_chunks.ordinal` of the winning chunk, 0-based.
    pub chunk_ordinal: i64,
}

/// Domain-qualified stable identity of one candidate, plus the content version
/// retrieval observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "domain", rename_all = "lowercase")]
pub enum CandidateIdentity {
    /// A `memories` row.
    Memory(MemoryIdentity),
    /// A `code_symbols` row.
    Code(CodeIdentity),
    /// A `documents` row and its winning chunk.
    Document(DocumentIdentity),
}

impl CandidateIdentity {
    /// Which corpus this identity addresses.
    pub fn domain(&self) -> CandidateDomain {
        match self {
            CandidateIdentity::Memory(_) => CandidateDomain::Memory,
            CandidateIdentity::Code(_) => CandidateDomain::Code,
            CandidateIdentity::Document(_) => CandidateDomain::Document,
        }
    }

    /// The content version: the digest or OID that changes when the candidate's
    /// content changes. A judgment's optional version pin is compared to this.
    pub fn content_version(&self) -> &str {
        match self {
            CandidateIdentity::Memory(m) => &m.content_hash,
            CandidateIdentity::Code(c) => &c.blob_oid,
            CandidateIdentity::Document(d) => &d.revision_hash,
        }
    }

    /// The domain-qualified reference string: `<domain>` followed by this
    /// domain's components in fixed order, each percent-escaped and joined with
    /// `:`. Total and injective — [`parse_ref`] inverts it exactly.
    pub fn candidate_ref(&self) -> String {
        let parts: Vec<String> = match self {
            CandidateIdentity::Memory(m) => vec![escape(&m.memory_id), escape(&m.content_hash)],
            CandidateIdentity::Code(c) => vec![
                escape(&c.repo),
                escape(&c.path),
                escape(&c.symbol),
                escape(&c.blob_oid),
            ],
            CandidateIdentity::Document(d) => vec![
                escape(&d.document_id),
                escape(&d.path),
                escape(&d.revision_hash),
                d.chunk_ordinal.to_string(),
            ],
        };
        format!("{}:{}", self.domain().as_str(), parts.join(":"))
    }
}

/// Parse a domain-qualified reference string back into a [`CandidateIdentity`].
///
/// An unknown domain, a component count that is not this domain's fixed arity,
/// a malformed `%XX` escape, a non-UTF-8 escape sequence, or a non-integer
/// chunk ordinal is an error naming the offending reference — never a silently
/// accepted candidate.
pub fn parse_ref(raw: &str) -> Result<CandidateIdentity> {
    // `%XX` escapes never contain `:`, so every literal colon is a separator.
    let mut parts = raw.split(':');
    let domain = parts.next().unwrap_or_default();
    let rest: Vec<&str> = parts.collect();
    match domain {
        "memory" => {
            let [id, hash] = arity(raw, &rest)?;
            Ok(CandidateIdentity::Memory(MemoryIdentity {
                memory_id: unescape(raw, id)?,
                content_hash: unescape(raw, hash)?,
            }))
        }
        "code" => {
            let [repo, path, symbol, oid] = arity(raw, &rest)?;
            Ok(CandidateIdentity::Code(CodeIdentity {
                repo: unescape(raw, repo)?,
                path: unescape(raw, path)?,
                symbol: unescape(raw, symbol)?,
                blob_oid: unescape(raw, oid)?,
            }))
        }
        "document" => {
            let [id, path, revision, ordinal] = arity(raw, &rest)?;
            Ok(CandidateIdentity::Document(DocumentIdentity {
                document_id: unescape(raw, id)?,
                path: unescape(raw, path)?,
                revision_hash: unescape(raw, revision)?,
                chunk_ordinal: ordinal.parse().map_err(|_| {
                    bad_ref(raw, &format!("chunk ordinal `{ordinal}` is not an integer"))
                })?,
            }))
        }
        other => Err(bad_ref(
            raw,
            &format!("unknown domain `{other}`: expected memory, code or document"),
        )),
    }
}

/// Require exactly `N` components after the domain, or error naming the
/// reference and both counts.
fn arity<'a, const N: usize>(raw: &str, rest: &[&'a str]) -> Result<[&'a str; N]> {
    <[&str; N]>::try_from(rest).map_err(|_| {
        bad_ref(
            raw,
            &format!("expected {N} components, found {}", rest.len()),
        )
    })
}

/// The single error shape for a malformed reference, so every failure names the
/// reference itself rather than only the component that broke.
fn bad_ref(raw: &str, why: &str) -> Error {
    Error::Config(format!("candidate ref `{raw}`: {why}"))
}

/// Percent-escape the reserved set — `:`, `%`, and every C0 control — one UTF-8
/// byte at a time, with uppercase hex digits. Everything else, `/` included,
/// stays literal so a path reads as a path.
fn escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch == ':' || ch == '%' || (ch as u32) < 0x20 {
            let mut buf = [0u8; 4];
            for byte in ch.encode_utf8(&mut buf).bytes() {
                out.push('%');
                out.push(hex_digit(byte >> 4));
                out.push(hex_digit(byte & 0x0f));
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// One uppercase hex digit for a nibble. A value above 15 cannot occur (both
/// call sites mask), and would render `0` rather than panic.
fn hex_digit(nibble: u8) -> char {
    char::from_digit(u32::from(nibble), 16)
        .unwrap_or('0')
        .to_ascii_uppercase()
}

/// Reverse [`escape`]: decode `%XX` back to bytes, then require the result to
/// be valid UTF-8.
fn unescape(raw: &str, token: &str) -> Result<String> {
    let mut bytes = Vec::with_capacity(token.len());
    let mut iter = token.bytes();
    while let Some(byte) = iter.next() {
        if byte != b'%' {
            bytes.push(byte);
            continue;
        }
        let (hi, lo) = iter
            .next()
            .zip(iter.next())
            .ok_or_else(|| bad_ref(raw, "truncated `%` escape"))?;
        bytes.push(hex_value(raw, hi)? << 4 | hex_value(raw, lo)?);
    }
    String::from_utf8(bytes).map_err(|_| bad_ref(raw, "escape sequence is not valid UTF-8"))
}

/// One hex digit's value, or an error naming the reference.
fn hex_value(raw: &str, digit: u8) -> Result<u8> {
    char::from(digit)
        .to_digit(16)
        .and_then(|v| u8::try_from(v).ok())
        .ok_or_else(|| bad_ref(raw, &format!("`{}` is not a hex digit", char::from(digit))))
}

#[cfg(test)]
#[path = "tests/candidate_identity.rs"]
mod tests;
