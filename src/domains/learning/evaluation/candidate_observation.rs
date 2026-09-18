//! The candidate observation contract: one candidate as retrieval produced it,
//! and the per-query envelope that makes the run reproducible.
//!
//! This is what #209 persists, #210 exports and #214 trains on. A change to any
//! field's presence or meaning is a change to [`OBSERVATION_VERSION`]. Contract:
//! `docs/designs/2026-09-18-domain-aware-retrieval-benchmark.md`.

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::domains::learning::evaluation::candidate_identity::CandidateIdentity;
use crate::utilities::digest::sha256_hex;

/// Version of the candidate observation contract. Bumped whenever a field is
/// added, removed, renamed, or given new semantics. A consumer that reads an
/// unknown version must refuse the record rather than guess.
pub const OBSERVATION_VERSION: u32 = 1;

/// Candidate text bounded to a predictable size, with the digest of the text
/// *before* bounding so a consumer can tell whether it holds the whole thing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedText {
    /// The text, truncated at a UTF-8 character boundary. Never split
    /// mid-character, so `text.len() <= max_bytes` always holds.
    pub text: String,
    /// `sha256` hex of the FULL source text, before truncation: the candidate
    /// text hash. Equal digests saw the same passage under different bounds.
    pub sha256: String,
    /// Byte length of the full source text, before truncation.
    pub full_bytes: usize,
    /// Whether `text` is shorter than the source text.
    pub truncated: bool,
}

impl BoundedText {
    /// Bound `source` to at most `max_bytes`, snapping down to the nearest
    /// character boundary, and digest the untruncated input.
    pub fn bound(source: &str, max_bytes: usize) -> Self {
        let full_bytes = source.len();
        let mut cut = max_bytes.min(full_bytes);
        while cut > 0 && !source.is_char_boundary(cut) {
            cut -= 1;
        }
        BoundedText {
            text: source.get(..cut).unwrap_or_default().to_string(),
            sha256: sha256_hex(source.as_bytes()),
            full_bytes,
            truncated: cut < full_bytes,
        }
    }

    /// The empty text a candidate whose row vanished between fusion and the
    /// text read gets. Dropping the candidate instead would silently shrink the
    /// pool, which is exactly what pool recall must not do.
    pub fn unavailable() -> Self {
        BoundedText::bound("", 0)
    }
}

/// Display and navigation fields. Deliberately separate from
/// [`CandidateIdentity`]: none of these may be used to match a judgment or a
/// training label.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateLocator {
    /// Human-readable headline.
    pub title: String,
    /// Repo label, where the domain has one.
    pub repo: Option<String>,
    /// File path, where the domain has one.
    pub path: Option<String>,
    /// 1-based inclusive line range, for code and document candidates. For a
    /// coalesced code chunk this is the WINNING CHUNK's range while the text is
    /// the parent symbol's, exactly as `comemory search-code` already reports.
    pub line_range: Option<(i64, i64)>,
    /// `" > "`-joined heading breadcrumb, document candidates only.
    pub heading_path: Option<String>,
    /// `code_symbols.id` at observation time: a recycled, session-scoped number
    /// usable to re-address `comemory search-code` output today, never an
    /// identity and never a training key.
    pub symbol_id: Option<i64>,
}

/// One candidate as retrieval produced it, for one query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateObservation {
    /// Domain plus stable identity plus content version.
    pub identity: CandidateIdentity,
    /// The domain-qualified reference string for `identity`, carried so a
    /// consumer never re-derives the encoding.
    pub candidate_ref: String,
    /// 1-based position in the candidate pool, in the order retrieval produced
    /// it, before any arm reorders anything.
    pub pool_position: usize,
    /// 1-based position on the displayed page, or `None` when the candidate was
    /// in the pool but below the page cut.
    pub returned_position: Option<usize>,
    /// The fused score retrieval assigned.
    pub retrieval_score: f64,
    /// 1-based position within this candidate's own domain leg, before fusion.
    pub rank_in_domain: usize,
    /// Lexical ladder tier for a memory candidate (1 strict, 2 word-OR,
    /// 3 subtoken-OR, 4 learned expansion); `None` for code and document.
    pub tier: Option<u8>,
    /// The candidate text, bounded.
    pub text: BoundedText,
    /// Where to find this candidate. Never used for matching.
    pub locator: CandidateLocator,
}

/// Lexical and BYO-vector runs are separate scenarios, never mixed in one set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum VectorScenario {
    /// No vector was supplied; the run is lexical only.
    Lexical,
    /// A caller-supplied vector, with the model identity that produced it.
    Supplied {
        /// Free-form embedder identity, e.g. `ollama:nomic-embed-text`.
        model: String,
        /// Vector dimension, which must match the `vec0` table's baked dim.
        dim: usize,
        /// `sha256` hex over the vector's little-endian `f32` bytes.
        digest: String,
    },
}

impl VectorScenario {
    /// The `Supplied` scenario for `vector`, digesting it so a replay can prove
    /// it used the same numbers.
    pub fn supplied(model: &str, vector: &[f32]) -> Self {
        let mut bytes = Vec::with_capacity(vector.len() * 4);
        for value in vector {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        VectorScenario::Supplied {
            model: model.to_string(),
            dim: vector.len(),
            digest: sha256_hex(&bytes),
        }
    }
}

/// Every filter one retrieval run applied, and nothing it did not. Each field
/// names the leg it narrows; the contract is per-leg, not "apply everything
/// everywhere".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectiveFilters {
    /// Domains in scope, ascending by their wire spelling.
    pub domains: Vec<String>,
    /// Repo label. Narrows the MEMORY and CODE legs; the document leg narrows
    /// by its own `documents.repo` column.
    pub repo: Option<String>,
    /// Canonical lowercase memory kind. Narrows the MEMORY leg ONLY.
    pub kind: Option<String>,
    /// Source language. Narrows the CODE leg ONLY.
    pub lang: Option<String>,
    /// Git-style path globs, OR'd. Narrow the DOCUMENT leg ONLY.
    pub path_globs: Vec<String>,
    /// Normalized ISO-8601 `--since` bound on `memories.created_at`.
    pub since: Option<String>,
    /// Normalized ISO-8601 `--until` bound. Filters candidates only.
    pub until: Option<String>,
    /// Normalized ISO-8601 `--as-of` bound. Filters candidates AND scopes the
    /// supersede penalty to superseders that existed at the cutoff.
    pub as_of: Option<String>,
    /// Lexical, or a caller-supplied vector with its model identity.
    pub vector: VectorScenario,
}

/// Every ranking knob that can move an order, flattened so `knobs_hash` is a
/// complete statement of the retrieval configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalKnobs {
    /// RRF fusion constant.
    pub rrf_k: f32,
    /// ACT-R decay exponent. `0.0` makes activation time-independent.
    pub decay: f64,
    /// MMR relevance-vs-diversity lambda.
    pub mmr_lambda: f64,
    /// Memory BM25 `(body, tags)` column weights.
    pub bm25_weights: (f32, f32),
    /// Code BM25 `(symbol, snippet, path_tokens)` column weights.
    pub code_bm25_weights: (f32, f32, f32),
    /// Graph-expansion hop depth.
    pub graph_hops: u32,
    /// Provisional top hits seeding the graph-expansion walk.
    pub graph_seeds: usize,
    /// Weighted-RRF contribution of the document leg.
    pub document_leg_weight: f32,
    /// Minimum cosine similarity for the memory vector leg.
    pub memory_threshold: f32,
    /// Minimum cosine similarity for the code vector leg.
    pub code_threshold: f32,
    /// SimHash Hamming radius for near-duplicate collapse.
    pub near_dup_hamming: u32,
    /// `(lo, hi)` bounds applied to every prior multiplier.
    pub prior_clamp: (f64, f64),
    /// Default page size.
    pub top_k: usize,
    /// Maximum depth pagination can reach into the ranked list.
    pub max_page_window: usize,
}

impl RetrievalKnobs {
    /// Read every knob off the effective config the run scored with.
    pub fn of(cfg: &Config) -> Self {
        RetrievalKnobs {
            rrf_k: cfg.retrieval.rrf_k,
            decay: cfg.rank.decay,
            mmr_lambda: cfg.rank.mmr_lambda,
            bm25_weights: cfg.retrieval.bm25_weights,
            code_bm25_weights: cfg.retrieval.code_bm25_weights,
            graph_hops: cfg.retrieval.graph_hops,
            graph_seeds: cfg.retrieval.graph_seeds,
            document_leg_weight: cfg.retrieval.document_leg_weight,
            memory_threshold: cfg.retrieval.memory_threshold,
            code_threshold: cfg.retrieval.code_threshold,
            near_dup_hamming: cfg.rank.near_dup_hamming,
            prior_clamp: cfg.rank.prior_clamp,
            top_k: cfg.retrieval.top_k,
            max_page_window: cfg.retrieval.max_page_window,
        }
    }

    /// Whether ACT-R activation is independent of wall-clock time for this knob
    /// set. Disabling access tracking alone does not achieve this; only a zero
    /// decay removes the `days_since` term from `score::activation`.
    pub fn decay_frozen(&self) -> bool {
        self.decay == 0.0
    }
}

/// One indexed repository's revision, from `repo_marker`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoRevision {
    /// Repo label.
    pub repo: String,
    /// `repo_marker.last_head` — HEAD commit at the last index.
    pub last_head: Option<String>,
    /// `repo_marker.last_indexed_at`, RFC 3339.
    pub last_indexed_at: Option<String>,
}

/// The pinned corpus / index snapshot. Equal `digest` means the same corpus at
/// the same index revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusSnapshot {
    /// Live (non-soft-deleted) `memories` rows.
    pub memories: u64,
    /// `code_symbols` rows.
    pub code_symbols: u64,
    /// `documents` rows.
    pub documents: u64,
    /// `document_chunks` rows.
    pub document_chunks: u64,
    /// Per-repo index revision, ascending by `repo`.
    pub repos: Vec<RepoRevision>,
    /// `sha256` hex over the canonical JSON of every field above.
    pub digest: String,
}

/// Which comemory, which schema, which knobs and which corpus a set of
/// observations was produced against.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalVersion {
    /// Version of the binary that produced the run.
    pub binary_version: String,
    /// The applied schema version (`store::migrate::CURRENT_VERSION`).
    pub schema_version: String,
    /// The pinned blend knobs the run scored with.
    pub knobs: RetrievalKnobs,
    /// `sha256` hex over the canonical JSON of `knobs`: the retrieval
    /// configuration version.
    pub knobs_hash: String,
    /// The corpus and index snapshot the run read.
    pub corpus: CorpusSnapshot,
}

/// Every candidate retrieval produced for ONE query, plus everything needed to
/// reproduce that query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryObservation {
    /// [`OBSERVATION_VERSION`] this record was written at.
    pub observation_version: u32,
    /// The `retrieval_log.query_id` when the originating run was tracked;
    /// `None` for an offline benchmark run, which writes no telemetry.
    pub query_id: Option<String>,
    /// The query text, verbatim as retrieval received it.
    pub query: String,
    /// Every filter that was in effect, per domain.
    pub filters: EffectiveFilters,
    /// Corpus, schema, binary and knob versions this run scored against.
    pub retrieval: RetrievalVersion,
    /// RFC 3339 UTC instant the run started.
    pub reference_time: String,
    /// Whether ACT-R activation was time-independent for this run. `false`
    /// means the numbers are a function of `reference_time`.
    pub decay_frozen: bool,
    /// Size of the ranked window the pool was taken from.
    pub pool_size: usize,
    /// Page size the `returned_position` values were cut at.
    pub page_limit: usize,
    /// Page offset. `0` for every benchmark run.
    pub page_offset: usize,
    /// The candidates, ascending by `pool_position`.
    pub candidates: Vec<CandidateObservation>,
}

/// `sha256` hex over a value's JSON — the digest behind `knobs_hash` and
/// `CorpusSnapshot::digest`.
///
/// Canonical only for map-free values: `serde_json` emits struct fields in
/// declaration order and sequences in order, but a `HashMap` serializes in
/// arbitrary order and would make the digest irreproducible. Both call sites
/// satisfy that — [`RetrievalKnobs`] is scalars and tuples, and
/// [`CorpusSnapshot`] is counters plus a `Vec<RepoRevision>` the store returns
/// ordered by repo — and a future field carrying a map must sort it into a
/// sequence before it reaches here.
pub fn canonical_digest<T: Serialize>(value: &T) -> crate::prelude::Result<String> {
    let json = serde_json::to_vec(value).map_err(crate::prelude::Error::Json)?;
    Ok(sha256_hex(&json))
}

#[cfg(test)]
#[path = "tests/candidate_observation.rs"]
mod tests;
