//! A reviewed relevance judgment, its target, and how a target matches a
//! candidate observation.
//!
//! The target is a plain struct with every key optional at the serde layer and
//! required by [`JudgmentTarget::resolve`] per domain: an internally tagged
//! enum cannot also carry `deny_unknown_fields`, and a silently ignored key in
//! a reviewed dataset is exactly the failure this contract exists to prevent.

use serde::{Deserialize, Serialize};

use crate::domains::learning::evaluation::candidate_identity::{
    CandidateDomain, CandidateIdentity,
};
use crate::prelude::*;

/// Highest relevance grade a judgment may carry. `gain = 2^relevance - 1`, so
/// an out-of-range grade would silently distort every nDCG figure.
pub const MAX_RELEVANCE: u8 = 3;

/// One reviewed judgment: how relevant a target is to its task's query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Judgment {
    /// Graded relevance, `0..=3`. `0` is an explicit "reviewed and not
    /// relevant", which is different from a candidate nobody judged.
    pub relevance: u8,
    /// What this judgment is about.
    pub target: JudgmentTarget,
}

/// A judgment's target as written in the set file: the human-writable stable
/// identity plus an optional content-version pin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgmentTarget {
    /// Which corpus the target lives in.
    pub domain: CandidateDomain,
    /// `memories.id`. Memory targets only.
    #[serde(default)]
    pub id: Option<String>,
    /// Repo label. Code targets only.
    #[serde(default)]
    pub repo: Option<String>,
    /// Repo-relative path for code, source-root-relative path for a document.
    #[serde(default)]
    pub path: Option<String>,
    /// Qualified symbol name. Code targets only.
    #[serde(default)]
    pub symbol: Option<String>,
    /// Version pin for a memory target.
    #[serde(default)]
    pub content_hash: Option<String>,
    /// Version pin for a code target.
    #[serde(default)]
    pub blob_oid: Option<String>,
    /// Version pin for a document target.
    #[serde(default)]
    pub revision_hash: Option<String>,
    /// Optional winning-chunk pin for a document target.
    #[serde(default)]
    pub chunk_ordinal: Option<i64>,
}

/// The validated, matchable form of a [`JudgmentTarget`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetKey {
    /// A memory, by id, optionally pinned to a body digest.
    Memory {
        /// `memories.id`.
        id: String,
        /// Optional `content_hash` pin.
        content_hash: Option<String>,
    },
    /// A code symbol, by its stable identity triple.
    Code {
        /// Repo label.
        repo: String,
        /// Repo-relative path.
        path: String,
        /// Qualified symbol name.
        symbol: String,
        /// Optional `blob_oid` pin.
        blob_oid: Option<String>,
    },
    /// A document, by its source-root-relative path.
    Document {
        /// Source-root-relative path.
        path: String,
        /// Optional `revision_hash` pin.
        revision_hash: Option<String>,
        /// Optional winning-chunk ordinal pin.
        chunk_ordinal: Option<i64>,
    },
}

/// Whether one target matches one observed identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchOutcome {
    /// The identity keys differ: a different candidate entirely.
    No,
    /// The identity keys agree and every pin holds.
    Yes,
    /// The identity keys agree but a pinned version does not: the judgment
    /// describes a content version this run did not see.
    Stale,
}

impl JudgmentTarget {
    /// Validate this target for `domain` and lift it into a [`TargetKey`].
    ///
    /// A required key that is absent, or a key belonging to another domain that
    /// is present, is an error naming `task` and the key — a target that can
    /// never match is a dataset bug, not a silent zero.
    pub fn resolve(&self, task: &str) -> Result<TargetKey> {
        match self.domain {
            CandidateDomain::Memory => {
                self.reject(
                    task,
                    &[
                        ("repo", self.repo.is_some()),
                        ("path", self.path.is_some()),
                        ("symbol", self.symbol.is_some()),
                        ("blob_oid", self.blob_oid.is_some()),
                        ("revision_hash", self.revision_hash.is_some()),
                        ("chunk_ordinal", self.chunk_ordinal.is_some()),
                    ],
                )?;
                Ok(TargetKey::Memory {
                    id: self.require(task, "id", self.id.as_deref())?,
                    content_hash: self.content_hash.clone(),
                })
            }
            CandidateDomain::Code => {
                self.reject(
                    task,
                    &[
                        ("id", self.id.is_some()),
                        ("content_hash", self.content_hash.is_some()),
                        ("revision_hash", self.revision_hash.is_some()),
                        ("chunk_ordinal", self.chunk_ordinal.is_some()),
                    ],
                )?;
                Ok(TargetKey::Code {
                    repo: self.require(task, "repo", self.repo.as_deref())?,
                    path: self.require(task, "path", self.path.as_deref())?,
                    symbol: self.require(task, "symbol", self.symbol.as_deref())?,
                    blob_oid: self.blob_oid.clone(),
                })
            }
            CandidateDomain::Document => {
                self.reject(
                    task,
                    &[
                        ("id", self.id.is_some()),
                        ("repo", self.repo.is_some()),
                        ("symbol", self.symbol.is_some()),
                        ("content_hash", self.content_hash.is_some()),
                        ("blob_oid", self.blob_oid.is_some()),
                    ],
                )?;
                Ok(TargetKey::Document {
                    path: self.require(task, "path", self.path.as_deref())?,
                    revision_hash: self.revision_hash.clone(),
                    chunk_ordinal: self.chunk_ordinal,
                })
            }
        }
    }

    /// A required key's value, or an error naming the task and the key.
    fn require(&self, task: &str, key: &str, value: Option<&str>) -> Result<String> {
        match value.map(str::trim).filter(|v| !v.is_empty()) {
            Some(v) => Ok(v.to_string()),
            None => Err(Error::Config(format!(
                "task `{task}`: a {} judgment target requires `{key}`",
                self.domain.as_str()
            ))),
        }
    }

    /// Refuse any key that belongs to another domain, naming the first one.
    fn reject(&self, task: &str, foreign: &[(&str, bool)]) -> Result<()> {
        match foreign.iter().find(|(_, present)| *present) {
            Some((key, _)) => Err(Error::Config(format!(
                "task `{task}`: `{key}` is not a key of a {} judgment target, so the \
                 target could never match",
                self.domain.as_str()
            ))),
            None => Ok(()),
        }
    }
}

impl TargetKey {
    /// Which corpus this key addresses.
    pub fn domain(&self) -> CandidateDomain {
        match self {
            TargetKey::Memory { .. } => CandidateDomain::Memory,
            TargetKey::Code { .. } => CandidateDomain::Code,
            TargetKey::Document { .. } => CandidateDomain::Document,
        }
    }

    /// Match this key against one observed identity. Identity keys decide
    /// `No` vs the rest; a pin that disagrees downgrades a hit to `Stale`.
    pub fn matches(&self, identity: &CandidateIdentity) -> MatchOutcome {
        match (self, identity) {
            (TargetKey::Memory { id, content_hash }, CandidateIdentity::Memory(m)) => outcome(
                *id == m.memory_id,
                pin_holds(content_hash.as_deref(), &m.content_hash),
            ),
            (
                TargetKey::Code {
                    repo,
                    path,
                    symbol,
                    blob_oid,
                },
                CandidateIdentity::Code(c),
            ) => outcome(
                *repo == c.repo && *path == c.path && *symbol == c.symbol,
                pin_holds(blob_oid.as_deref(), &c.blob_oid),
            ),
            (
                TargetKey::Document {
                    path,
                    revision_hash,
                    chunk_ordinal,
                },
                CandidateIdentity::Document(d),
            ) => outcome(
                *path == d.path,
                pin_holds(revision_hash.as_deref(), &d.revision_hash)
                    && chunk_ordinal.is_none_or(|o| o == d.chunk_ordinal),
            ),
            _ => MatchOutcome::No,
        }
    }
}

/// Fold "identity agrees" and "pins hold" into one [`MatchOutcome`].
fn outcome(identity_agrees: bool, pins_hold: bool) -> MatchOutcome {
    match (identity_agrees, pins_hold) {
        (false, _) => MatchOutcome::No,
        (true, true) => MatchOutcome::Yes,
        (true, false) => MatchOutcome::Stale,
    }
}

/// An absent pin always holds; a present one must equal the observed version.
fn pin_holds(pin: Option<&str>, observed: &str) -> bool {
    pin.is_none_or(|p| p == observed)
}

#[cfg(test)]
#[path = "tests/judgment.rs"]
mod tests;
