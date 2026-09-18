# Domain-aware retrieval benchmark and candidate observation contract — Design

**Date:** 2026-09-18 **Status:** Approved **Author:** Falconiere R. Barbosa
**Topic:** A versioned offline benchmark over real reviewed queries and a pinned
corpus snapshot, and the shared candidate observation contract every later
reranking issue consumes.

Tracking issue: [#207](https://github.com/Falconiere/comemory/issues/207).
This issue: [#208](https://github.com/Falconiere/comemory/issues/208).

## Problem

`comemory eval` measures one thing: lexical memory retrieval. Its golden format
is a list of memory ids per query
(`src/domains/learning/evaluation/golden.rs`), its runner pins
`Domains::memory_only()` and the memory-only `pipeline::search`
(`src/domains/learning/evaluation/runner.rs`), and its metrics are recall@k and
MRR over that one domain (`src/domains/learning/evaluation/metrics.rs`).

Three consequences block the reranking roadmap:

1. **No code, document or mixed-domain signal.** `comemory find` returns three
   domains, but nothing scores those rankings, so no reranking change to them
   can be shown to help or hurt.
2. **No candidate-pool visibility.** A miss today is indistinguishable between
   "retrieval never produced the right result" and "retrieval produced it and
   ranked it below the cut". A reranker can only fix the second. Without the
   split, a reranking experiment cannot be attributed.
3. **No durable candidate record.** `retrieval::find::track_run` logs only
   memory ids even for a three-domain run, `store::retrieval_log` stores ids and
   a partial filter set but no candidate text, and `fuse_domains::UnifiedHit`
   drops each leg's passage before a caller sees it. Historical logs therefore
   cannot reconstruct a versioned training example, which is why #209 (persist),
   #210 (export), #213 (materialize at query time) and #214 (train) all need a
   contract defined once, here, before any of them writes a row.

## Non-Goals

1. **No live capture.** Nothing in this change writes a candidate observation to
   SQLite or to `retrieval_log`. Persistence is #209.
2. **No dataset export.** No training-set file format, no split logic. That is
   #210.
3. **No reranking in the retrieval path.** `comemory search` / `search-code` /
   `find` / `context` behave byte-identically after this change. Wiring a scorer
   into retrieval is #213.
4. **No subprocess protocol, runner, or timeouts.** The versioned process
   protocol and its bounded runner are #211. This design fixes only the
   *meaning* of a candidate identity and how candidate text is bounded, so
   #211's opaque domain-qualified token and this contract agree.
5. **No model, no Python, no inference backend.** #212 supplies the reference
   scorer. This change provides the arm-comparison machinery it will plug into,
   through a scores file, not a process.
6. **No change to the existing memory evaluation.** `comemory eval`, the
   `GoldenPair` YAML format, `EvalReport`'s field set, the `eval_runs` table and
   every historical metric stay exactly as they are. The benchmark is a new,
   separate surface.
7. **No new SQLite table, column, or migration.** The benchmark reads; its only
   write is the report file the operator names.

## Architecture

### The shape

A new CLI-only subcommand, `comemory benchmark`, drives a **versioned dataset
file** of manually reviewed tasks against the real retrieval legs, captures a
**candidate observation** per candidate, scores one or more **arms** over that
one captured snapshot, and emits a **replayable artifact**.

```text
benchmark set (YAML, reviewed)     pinned ranking knobs (TuneCandidate)
            |                                   |
            +-----------------+-----------------+
                              v
        retrieval::unified::run_legs  (track: false, no telemetry at all)
                              v
     memory Reranked | code CodeReranked | document DocHit   <- rich rows, text intact
                              v
        fuse_domains::fuse  ->  candidate pool, in pipeline order
                              v
              CandidateObservation per candidate  (the contract)
                              v
        arm "deterministic"      arm(s) from --scores <file>
                              v
   pool recall | recall@k | MRR | nDCG@k | paired bootstrap | per domain
                              v
              summary on stdout, full artifact to --report <path>
```

### Decisive trade-offs

**Reuse the real legs, do not re-implement them.** `unified::find` already runs
the three legs and fuses them, but it returns `UnifiedHit`, which has deliberately
dropped each leg's passage text and version anchors. Rather than add a second
retrieval path, `unified::find` is split into two public halves — `run_legs`,
which returns the legs' own reranked rows, and `fuse_legs`, which fuses them —
leaving `find` as exactly `run_legs` + `fuse_legs` + `pipeline::paginate` with
byte-identical output. The benchmark calls `run_legs`, reads identity and text
off the rich rows, then calls **the same `fuse_legs`**, so there is no second
fusion to drift from. A colocated test asserts the benchmark's pool prefix
equals `find`'s page for the same inputs, and every run records
`page_matches_production` per task.

**The candidate pool is the full in-window ranked list; the page is its prefix.**
The benchmark runs one query per task with `PageWindow { offset: 0, limit: 0 }`,
which `pipeline::pool_size` resolves to `max_page_window`: the whole ranked
window. Pool recall is measured over that list. The deterministic arm's page is
its first `k` entries. Both numbers therefore come from **one** snapshot, which
is what makes an arm comparison paired and what lets a reranker be evaluated
against exactly the candidates it would have seen. The benchmark additionally
issues the production-shaped page query (`limit = k`) once per task and records
`page_matches_production`, so the prefix-stability property `pipeline::pool_size`
claims is measured rather than assumed. A `false` there is information, not a
failure: the production pool for a `k`-sized page is
`clamp(2k, CANDIDATE_POOL, max_page_window)` — `50` at the defaults —
while the benchmark's is `max_page_window`, and MMR min-max-normalizes
relevance across whatever pool it is given, so a wider pool can legitimately
move a deeper pick. Recording it is how that effect becomes visible instead of
being asserted away.

**Tracking off is not enough; decay is pinned.** `SearchOptions { track: false }`
suppresses `record_access` and the `retrieval_log` write, but ACT-R activation
still reads the wall clock: `rerank::rerank` builds `PoolCtx { now:
OffsetDateTime::now_utc(), .. }` and `code_prior::priors` calls
`score::days_since(last_accessed, now)`. Disabling access tracking does not
freeze activation decay. The benchmark takes the stronger route available
without touching production ranking: **the dataset must declare a `ranking:`
block, and its `decay` is the pinned ACT-R decay exponent.** At `decay = 0.0`,
`score::activation` reduces to `ln(max(access_count, 1))`, which contains no
time term at all, so the run is reproducible across days. The report records
`reference_time` and `decay_frozen` either way, and a non-zero decay is reported
as a run whose numbers are a function of `reference_time`. `run_legs` writes no
telemetry of any kind, so `track` is structurally false rather than a flag
someone can flip.

**The ranking block is a `TuneCandidate`.** The six blend knobs are already a
declared, validated type (`evaluation::tune::TuneCandidate`) with a validating
constructor (`tune::validated_with_candidate`), already used by
`eval::effective_config` for the console's "what would these knobs have scored?"
run. Reusing it makes the dataset's pinned retrieval configuration the same
shape the rest of the learning loop already speaks, and an out-of-range value a
`BadRequest` rather than a scored run.

**Arms compare over one snapshot, and a scorer arrives as a file.** #212's
reference scorer does not exist yet and #211 owns the process protocol, so the
benchmark accepts arm scores as a JSON file keyed by task id and candidate ref.
That is the replay loop: run the benchmark once with `--report`, hand the
artifact's bounded candidate text to any scorer, feed its scores back with
`--scores`. No subprocess, no timeout, no protocol is defined here.

### Where the code lives

| Path | Role |
| --- | --- |
| `src/domains/learning/evaluation/candidate_identity.rs` | `CandidateDomain`, `CandidateIdentity` and the candidate-ref string codec |
| `src/domains/learning/evaluation/candidate_observation.rs` | `CandidateObservation`, `BoundedText`, `EffectiveFilters`, `RetrievalVersion`, `QueryObservation` |
| `src/domains/learning/evaluation/judgment.rs` | `Judgment`, `JudgmentTarget`, `TargetKey`, and the match / stale / no outcome |
| `src/domains/learning/evaluation/benchmark_set.rs` | `BenchmarkSet` / `BenchmarkTask` / `Budgets`, YAML load and validation |
| `src/domains/learning/evaluation/task_filters.rs` | `TaskFilters` and the rule that refuses a filter whose leg the task does not run |
| `src/domains/learning/evaluation/candidate_facts.rs` | identity, content version and bounded text read off the leg rows, built at one site per domain |
| `src/domains/learning/evaluation/benchmark_runner.rs` | one task → a captured pool + matched judgments, over `unified::run_legs` |
| `src/domains/learning/evaluation/benchmark_observe.rs` | the fused pool → `CandidateObservation` list and `EffectiveFilters` record |
| `src/domains/learning/evaluation/benchmark_metrics.rs` | pool recall, recall@k, MRR, nDCG@k, paired bootstrap |
| `src/domains/learning/evaluation/benchmark_arm.rs` | the baseline and scored arms, the scores file, the total ordering rule |
| `src/domains/learning/evaluation/benchmark_arm_report.rs` | one arm's metrics, latency, paired delta and budget verdict |
| `src/domains/learning/evaluation/benchmark_report.rs` | artifact assembly and the `summary()` shape `--json` prints |
| `src/domains/learning/evaluation/run_environment.rs` | hardware, memory, working set, and the pinned corpus/index snapshot |
| `src/domains/learning/benchmark.rs` | the `Request` / `run` command core |
| `src/cli/benchmark.rs` | clap surface, artifact write, TTY and `--json` rendering |
| `src/store/code_text.rs` | batched `code_symbols` read: `blob_oid`, snippet, line range, identity |
| `src/domains/retrieval/unified.rs` | `run_legs` and `fuse_legs` extracted from `find`, and `UnifiedQuery` (no behavior change) |

`domains::learning -> domains::retrieval` is already a declared owner dependency
in `scripts/architecture-policy.json`; no policy edge is added. All SQL stays in
`src/store/`. No `rusqlite` name leaves `store/`.

## Candidate observation contract

> This section is the contract #209 persists, #210 exports, #211 tokenizes,
> #213 materializes at query time and #214 trains on. It is normative. A change
> to it is a change to `OBSERVATION_VERSION`.

### Contract version

```rust
/// Version of the candidate observation contract. Bumped whenever a field is
/// added, removed, renamed, or given new semantics.
pub const OBSERVATION_VERSION: u32 = 1;
```

Every serialized envelope carries `observation_version`. A consumer that reads
an unknown version must refuse the record rather than guess.

### Domain

```rust
/// The corpus a candidate came from. The string forms are exactly the
/// `fuse_domains::DOMAIN_*` labels already carried on `UnifiedHit::domain`.
pub enum CandidateDomain {
    /// `"memory"` — hand-authored markdown in `memories`.
    Memory,
    /// `"code"` — an extracted symbol in `code_symbols`.
    Code,
    /// `"document"` — an indexed external document in `documents`.
    Document,
}
```

`CandidateDomain::as_str` returns `"memory"`, `"code"` or `"document"`. These are
the same three literals `fuse_domains::DOMAIN_MEMORY` / `DOMAIN_CODE` /
`DOMAIN_DOCUMENT` already define; the contract does not mint a second vocabulary.

### Stable identity and content version

Identity is the pair "what is this thing" plus "which version of it did
retrieval show". The two are separate fields per domain: the identity keys are
stable across re-indexing, the version key changes when the content changes.

```rust
/// Domain-qualified stable identity of one candidate, plus the content
/// version retrieval observed.
pub enum CandidateIdentity {
    Memory(MemoryIdentity),
    Code(CodeIdentity),
    Document(DocumentIdentity),
}

/// A memory candidate.
pub struct MemoryIdentity {
    /// `memories.id` — the 8-hex prefix of `sha256(body.trim_end())`.
    pub memory_id: String,
    /// Content version: the 64-hex `sha256(body.trim_end())` of the body
    /// retrieval returned, equal by construction to `memories.content_hash`
    /// and to the frontmatter `content_hash` field.
    pub content_hash: String,
}

/// A code candidate.
pub struct CodeIdentity {
    /// `code_symbols.repo` — the repo label.
    pub repo: String,
    /// `code_symbols.path` — repo-relative path.
    pub path: String,
    /// `code_symbols.symbol` — the qualified symbol name. For a coalesced
    /// cAST chunk this is the PARENT symbol's name, matching the identity
    /// `code_feedback` is keyed by and the join
    /// `retrieval::code_prior::signals` performs.
    pub symbol: String,
    /// Content version: `code_symbols.blob_oid`, the git blob OID of the file
    /// at index time. Never the `code_symbols` rowid — re-indexing purges and
    /// reinserts a touched file's rows and SQLite recycles the freed rowids.
    pub blob_oid: String,
}

/// A document candidate. The parent document is the identity; the winning
/// chunk is the passage that was actually matched.
pub struct DocumentIdentity {
    /// `documents.id` — the 32-hex parent document id. Derived, and not
    /// hand-writable.
    pub document_id: String,
    /// `source_files.relative_path` — the parent's human-writable stable
    /// name. Part of the IDENTITY, not the locator, precisely so a reviewed
    /// judgment can address a document without matching on a display field.
    pub path: String,
    /// Content version: `documents.revision_hash`, the parent's content
    /// identity for rename / edit detection.
    pub revision_hash: String,
    /// `document_chunks.ordinal` — the 0-based ordinal of the winning chunk
    /// within the parent, i.e. which passage this candidate's text is.
    pub chunk_ordinal: i64,
}
```

**Why `(repo, path, symbol)` and not `code_symbols.id`.** The rowid is the id
`search-code` prints and the id `comemory feedback --used-code` accepts, but it
is recycled by re-indexing; `domains::learning::code_feedback` already keys its
counters on `(repo, path, symbol)` for exactly this reason and refuses to join
historical rowids to today's symbols. A training example keyed on a rowid would
silently re-attribute to whatever symbol inherits the number. The observation
still *carries* the rowid, as a non-identity locator (see `CandidateLocator`),
so a caller can round-trip to `search-code` output within one session.

### The domain-qualified candidate reference string

#211 treats a candidate id as an opaque domain-qualified string. This is that
string. It is total, injective, and reversible.

```text
candidate_ref := domain ":" component *( ":" component )
domain        := "memory" / "code" / "document"
component     := *( unescaped / escaped )
unescaped     := any Unicode scalar value except ":" , "%" , and U+0000..U+001F
escaped       := "%" HEXDIG HEXDIG   ; one UTF-8 byte, UPPERCASE hex digits
```

Component order and arity are fixed per domain:

| Domain | Components, in order | Arity |
| --- | --- | --- |
| `memory` | `memory_id`, `content_hash` | 2 |
| `code` | `repo`, `path`, `symbol`, `blob_oid` | 4 |
| `document` | `document_id`, `path`, `revision_hash`, `chunk_ordinal` | 4 |

Parsing splits on unescaped `:`, takes the first token as the domain, and
requires the remaining token count to equal that domain's arity exactly. A wrong
arity, an unknown domain, an invalid escape, or a non-integer `chunk_ordinal` is
a parse error naming the offending reference — never a silently accepted
candidate.

Examples:

```text
memory:5a9f19bc:5a9f19bc403e9aa0752a777b56073d1aed3a367fc78d25d9631b0be5ccaeb702
code:comemory:src%2Fdomains%2Fretrieval%2Frerank.rs:rerank:9d1c1f0b8a4e...
document:0f1e2d3c4b5a69788796a5b4c3d2e1f0:guides/schema-migrations.md:7b2a9c...:3
```

`/` is not in the escaped set, so a path appears literally; the escape above is
shown only to illustrate that escaping is per-component and round-trips. The
implementation escapes exactly `:`, `%` and C0 controls, so
`code:comemory:src/domains/retrieval/rerank.rs:rerank:<oid>` is the real
encoding of that example.

### Bounded text

```rust
/// The candidate text handed to a scorer, bounded so an observation has a
/// predictable size, plus the digest of the text BEFORE bounding so a
/// consumer can tell whether it is holding the whole thing.
pub struct BoundedText {
    /// The text, truncated to at most `max_bytes` at a UTF-8 character
    /// boundary. Never split mid-character.
    pub text: String,
    /// `sha256` hex of the FULL source text, before truncation. This is the
    /// candidate text hash; two observations with equal `sha256` saw the same
    /// passage even if one was bounded more tightly.
    pub sha256: String,
    /// Byte length of the full source text, before truncation.
    pub full_bytes: usize,
    /// Whether `text` is shorter than the full source text.
    pub truncated: bool,
}
```

Per-domain source text, which is exactly what retrieval already matched on:

| Domain | Source text |
| --- | --- |
| `memory` | `memories.body`, carried on `rerank::Reranked::body` |
| `code` | `code_symbols.snippet` of the row at `CodeReranked::symbol_id` |
| `document` | `document_chunks.text` of the winning chunk, carried on `doc_route::DocHit::snippet` |

The bound is `max_text_bytes`, declared by the dataset (default 4096). It is a
byte bound applied after UTF-8 boundary snapping, so `text.len() <=
max_text_bytes` always holds.

**One deliberate asymmetry, for code.** `code_rerank::coalesce` folds a cAST
chunk onto its parent: the resulting `CodeReranked` carries the PARENT's
`symbol_id`, repo, path and symbol, but the WINNING CHUNK's `line_start` /
`line_end`. The observation follows that split exactly — identity and
bounded text come from the parent row (the unit `code_feedback` is keyed by and
the unit a scorer should judge), while `locator.line_range` stays the winning
chunk's, which is what `comemory search-code` already prints. The two are
therefore allowed to disagree, and the field docs say so rather than letting a
consumer assume the text spans the line range.

### One candidate

```rust
/// One candidate as retrieval produced it, for one query.
pub struct CandidateObservation {
    /// Domain plus stable identity plus content version.
    pub identity: CandidateIdentity,
    /// The domain-qualified reference string for `identity`, carried so a
    /// consumer never has to re-derive the encoding.
    pub candidate_ref: String,
    /// 1-based position in the candidate pool, in the order retrieval
    /// produced it, before any arm reorders anything. This is the
    /// "original candidate order".
    pub pool_position: usize,
    /// 1-based position on the displayed page, or `None` when this candidate
    /// was in the pool but below the page cut. The pool and the page are
    /// different sets and this field is what distinguishes them.
    pub returned_position: Option<usize>,
    /// The fused score retrieval assigned (`UnifiedHit::score`).
    pub retrieval_score: f64,
    /// 1-based position within this candidate's own domain leg, before
    /// fusion (`UnifiedHit::rank_in_domain`).
    pub rank_in_domain: usize,
    /// Lexical ladder tier for a memory candidate (1 strict, 2 word-OR,
    /// 3 subtoken-OR, 4 learned expansion); `None` for code and document,
    /// whose legs do not run the memory ladder.
    pub tier: Option<u8>,
    /// The candidate text, bounded.
    pub text: BoundedText,
    /// Non-identity locators: where to find this candidate in a UI or an
    /// editor. Never used for matching.
    pub locator: CandidateLocator,
}

/// Display and navigation fields. Deliberately separate from
/// `CandidateIdentity`: none of these may be used to match a judgment or a
/// training label.
pub struct CandidateLocator {
    /// Human-readable headline (`UnifiedHit::title`).
    pub title: String,
    /// Repo label where the domain has one.
    pub repo: Option<String>,
    /// File path where the domain has one — repo-relative for code, source-root
    /// relative for a document, data-dir relative for a memory's markdown.
    pub path: Option<String>,
    /// 1-based inclusive line range for code and document candidates.
    pub line_range: Option<(i64, i64)>,
    /// `" > "`-joined heading breadcrumb, document candidates only.
    pub heading_path: Option<String>,
    /// `code_symbols.id` of a code candidate at observation time. A recycled,
    /// session-scoped number: usable to re-address `comemory search-code`
    /// output today, never as an identity or a training key.
    pub symbol_id: Option<i64>,
}
```

### The per-query envelope

```rust
/// Every candidate retrieval produced for ONE query, plus everything needed
/// to reproduce that query.
pub struct QueryObservation {
    /// `OBSERVATION_VERSION` this record was written at.
    pub observation_version: u32,
    /// The `retrieval_log.query_id` when the originating run was tracked;
    /// `None` for an offline benchmark run, which never writes telemetry.
    pub query_id: Option<String>,
    /// The query text, verbatim as retrieval received it.
    pub query: String,
    /// Every filter that was in effect, per domain.
    pub filters: EffectiveFilters,
    /// Corpus, schema, binary and knob versions this run scored against.
    pub retrieval: RetrievalVersion,
    /// RFC 3339 UTC instant the run started. Ranking is independent of it
    /// when `retrieval.knobs.decay` is `0.0`; see `decay_frozen`.
    pub reference_time: String,
    /// Whether ACT-R activation was time-independent for this run
    /// (`knobs.decay == 0.0`). `false` means the numbers are a function of
    /// `reference_time` and are not comparable across days.
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
```

### Complete effective filters

Each filter narrows specific legs. The contract records all of them, `null`
where unset, and the per-leg meaning is fixed rather than "apply everything
everywhere":

```rust
/// Every filter one retrieval run applied, and nothing it did not.
pub struct EffectiveFilters {
    /// Domains in scope, ascending by the `CandidateDomain` string form.
    /// Mirrors `retrieval::scope::Domains`.
    pub domains: Vec<String>,
    /// Repo label. Narrows the MEMORY and CODE legs. The document leg
    /// narrows by its own `documents.repo` column inside `doc_route`.
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

/// Lexical and BYO-vector runs are separate scenarios and never mixed in one
/// benchmark set.
pub enum VectorScenario {
    /// No vector was supplied; the run is lexical only.
    Lexical,
    /// A caller-supplied vector, with the model identity that produced it.
    Supplied {
        /// Free-form embedder identity, e.g. `ollama:nomic-embed-text`.
        model: String,
        /// Vector dimension, which must match the `vec0` table's baked dim.
        dim: usize,
        /// `sha256` hex over the vector's little-endian `f32` bytes, so a
        /// replay can prove it used the same vector.
        digest: String,
    },
}
```

### Retrieval and corpus version

```rust
/// Which comemory, which schema, which knobs, and which corpus snapshot a
/// set of observations was produced against.
pub struct RetrievalVersion {
    /// `env!("CARGO_PKG_VERSION")` of the binary that produced the run.
    pub binary_version: String,
    /// `store::migrate::CURRENT_VERSION` — the applied schema version.
    pub schema_version: String,
    /// The pinned blend knobs the run scored with.
    pub knobs: RetrievalKnobs,
    /// `sha256` hex over the canonical JSON of `knobs`. This is the
    /// "retrieval configuration version": two runs with equal `knobs_hash`
    /// used the same ranking configuration.
    ///
    /// Canonical here means map-free. `serde_json` emits struct fields in
    /// declaration order and sequences in order, but a `HashMap` serializes in
    /// arbitrary order and would make the digest irreproducible. Both digested
    /// values satisfy that today — `RetrievalKnobs` is scalars and tuples,
    /// `CorpusSnapshot` is counters plus a `Vec<RepoRevision>` the store
    /// returns ordered by repo — and a consumer adding a field that carries a
    /// map must sort it into a sequence before digesting.
    pub knobs_hash: String,
    /// The corpus and index snapshot the run read.
    pub corpus: CorpusSnapshot,
}

/// Every ranking knob that can move an order, flattened so `knobs_hash` is
/// a complete statement of the configuration.
pub struct RetrievalKnobs {
    pub rrf_k: f32,
    pub decay: f64,
    pub mmr_lambda: f64,
    pub bm25_weights: (f32, f32),
    pub code_bm25_weights: (f32, f32, f32),
    pub graph_hops: u32,
    pub graph_seeds: usize,
    pub document_leg_weight: f32,
    pub memory_threshold: f32,
    pub code_threshold: f32,
    pub near_dup_hamming: u32,
    pub prior_clamp: (f64, f64),
    pub top_k: usize,
    pub max_page_window: usize,
}

/// The pinned corpus / index snapshot. `digest` is the snapshot identity:
/// two runs with equal `digest` read the same corpus at the same index
/// revision.
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

/// One indexed repository's revision, from `repo_marker`.
pub struct RepoRevision {
    /// Repo label.
    pub repo: String,
    /// `repo_marker.last_head` — HEAD commit at the last index, the
    /// repository/index revision. `None` for a repo indexed outside git.
    pub last_head: Option<String>,
    /// `repo_marker.last_indexed_at`, RFC 3339.
    pub last_indexed_at: Option<String>,
}
```

### What a consumer must guarantee

1. **Never insert a positive that retrieval did not return.** An observation set
   is a record of what retrieval produced. A judged-relevant item missing from
   the pool is a recall miss, reported as such, and must not be appended to the
   candidates.
2. **Never match on a locator.** Judgments, labels and joins match on
   `CandidateIdentity` (and, when pinned, on the content version). Titles,
   paths, and `symbol_id` are display only.
3. **Refuse an unknown `observation_version`.**
4. **Treat a content-version mismatch as stale, not as a match.** A judgment or
   label that pins a content version and meets a different one is excluded and
   counted, never silently accepted.

## Interfaces / Schema

### Benchmark set (YAML, reviewed, versioned)

```yaml
version: 1
name: comemory-mixed-v1
description: Reviewed mixed-domain tasks over the pinned fixture corpus.

# Pinned retrieval configuration. Required — a benchmark that inherits the
# live config is not reproducible. Shape is evaluation::tune::TuneCandidate,
# validated by tune::validated_with_candidate.
ranking:
  rrf_k: 60.0
  decay: 0.0            # 0.0 freezes ACT-R activation; see decay_frozen
  mmr_lambda: 0.7
  bm25_weights: [1.0, 3.0]
  graph_hops: 2
  graph_seeds: 8

# Optional. Absent => lexical scenario. Present => every task MUST supply a
# vector of exactly `dim` floats, and a task without one is a load error.
# vectors:
#   model: "ollama:nomic-embed-text"
#   dim: 1024

defaults:
  k: 5                  # recall@k / nDCG@k cut
  max_text_bytes: 4096  # BoundedText bound

budgets:
  min_tasks: 8                    # fewer judged tasks => Inconclusive
  min_ndcg_gain: 0.02             # improvement threshold, on the paired CI
  max_ndcg_regression: 0.01       # regression threshold, on the paired CI
  max_p95_task_ms: 1500           # latency budget per task

tasks:
  - id: mem-ranking-01
    domain: memory               # memory | code | document | all
    query: "paged search non-determinism"
    filters:
      repo: comemory
      kind: bug
    judgments:
      - relevance: 3             # 0..=3; 0 is an explicit non-relevant
        target: { domain: memory, id: 5a9f19bc }

  - id: code-rerank-01
    domain: code
    query: "rerank priors activation"
    filters:
      lang: rust
    judgments:
      - relevance: 3
        target:
          domain: code
          repo: comemory
          path: src/domains/retrieval/rerank.rs
          symbol: rerank

  - id: doc-chunk-01
    domain: document
    query: "schema migrations journal"
    filters:
      path: ["docs/guides/*.md"]
    judgments:
      - relevance: 2
        target: { domain: document, path: guides/schema-migrations.md }

  - id: mixed-01
    domain: all
    query: "activation decay"
    judgments:
      - relevance: 3
        target: { domain: memory, id: 5a9f19bc }
      - relevance: 1
        target:
          domain: code
          repo: comemory
          path: src/domains/retrieval/score.rs
          symbol: activation
```

A judgment target carries the **human-writable stable identity** and an
**optional content-version pin**:

| `target.domain` | Required | Optional version pin | Rejected keys |
| --- | --- | --- | --- |
| `memory` | `id` | `content_hash` | `repo`, `symbol`, `path`, `blob_oid`, `revision_hash`, `chunk_ordinal` |
| `code` | `repo`, `path`, `symbol` | `blob_oid` | `id`, `content_hash`, `revision_hash`, `chunk_ordinal` |
| `document` | `path` (source-root relative) | `revision_hash`, `chunk_ordinal` | `id`, `repo`, `symbol`, `content_hash`, `blob_oid` |

The deserialized shape is a **plain struct**, not a serde-tagged enum:

```rust
/// A judgment's target, as written in the set file. Every key is optional at
/// the serde layer and required by `validate` per domain, because an
/// internally tagged enum cannot also carry `deny_unknown_fields` — and a
/// silently ignored key in a reviewed dataset is exactly the failure this
/// contract exists to prevent.
#[serde(deny_unknown_fields)]
pub struct JudgmentTarget {
    pub domain: CandidateDomain,
    pub id: Option<String>,
    pub repo: Option<String>,
    pub path: Option<String>,
    pub symbol: Option<String>,
    pub content_hash: Option<String>,
    pub blob_oid: Option<String>,
    pub revision_hash: Option<String>,
    pub chunk_ordinal: Option<i64>,
}

/// The validated, matchable form. `JudgmentTarget::resolve` produces it or
/// errors naming the task id, the domain and the offending key.
pub enum TargetKey {
    Memory { id: String, content_hash: Option<String> },
    Code { repo: String, path: String, symbol: String, blob_oid: Option<String> },
    Document { path: String, revision_hash: Option<String>, chunk_ordinal: Option<i64> },
}
```

Matching a judgment to an observation: the domain and every required key must be
equal. A required key that is absent, or a key belonging to another domain that
is present, is a load error naming the task id and the key — never a target
that silently matches nothing. If a version pin is present and differs from the
observation's content version, the judgment is **stale**: excluded from `Jl` and
`Jp` entirely and counted in `judgments_stale`.

A document judgment addresses its target by the source-root-relative path
because the 32-hex `documents.id` is derived and not hand-writable. That path is
therefore a field of `DocumentIdentity`, **not** of `CandidateLocator` —
otherwise matching a document judgment would break this contract's own rule that
nothing matches on a locator. The observation still carries the derived id and
the `revision_hash` a judgment may pin.

### CLI

```text
comemory benchmark --set <FILE> [--report <FILE>] [--scores <FILE>]... [--k <N>] [--json]
```

| Flag | Default | Effect |
| --- | --- | --- |
| `--set` | required | Path to the benchmark set YAML |
| `--report` | unset | Write the full replayable artifact JSON here (creates/truncates) |
| `--scores` | unset, repeatable | Add one arm from a scores file |
| `--k` | set's `defaults.k` | Override the recall@k / nDCG@k cut |

`comemory benchmark` is CLI-only: it writes a report file to an
operator-named filesystem path, which is what already puts `install` and
`setup` in `serve::routes::meta::CLI_ONLY`. It is added to that list and to
`tests/api__parity.rs`'s independently-stated mirror.

TTY output is one summary block per arm plus the per-domain table. `--json`
prints the artifact with every `tasks[].observation.candidates` array emptied
— the summary, which stays small enough to pipe — while `--report` writes
the artifact with them intact. Every other field is identical between the two,
so a consumer never has to reconcile two shapes.

Exit codes follow `sysexits.h` through the existing `Error` mapping in
`src/main.rs`, and the benchmark adds no new variant. A malformed set file, an
invalid scores file, or an out-of-bounds `ranking:` block is `Error::Config`
— `EX_CONFIG` (78), the same code `comemory eval --golden <broken file>`
already returns, since `golden::load_file` reports through the same variant. A
set that resolves zero tasks is `Error::Unavailable` — `EX_UNAVAILABLE`
(69), matching `golden::resolve`'s empty-set refusal. Clap's own argument
errors stay `EX_USAGE` (64). The `ranking:` block's own validation failure
arrives from `tune::validated_with_candidate` as `Error::BadRequest`; the
loader **converts** it into `Error::Config` naming the set file, because from
the operator's point of view the fault is in the file they wrote, not in a
request.

### Scores file (an arm)

```json
{
  "arm": "base-cross-encoder",
  "scorer_version": "bge-reranker-base@1",
  "scores": {
    "mem-ranking-01": {
      "memory:5a9f19bc:5a9f19bc40...": 8.12,
      "memory:76a8b36c:76a8b36c91...": 2.05
    }
  }
}
```

Ordering rule for a scored arm: candidates with a supplied score come first,
descending by score, ties broken by ascending `pool_position`; candidates with no
supplied score keep their relative pool order and follow. The arm records
`scored_fraction` so a partially-scored arm is visible rather than silently
compared as if complete.

### Report artifact JSON

```json
{
  "artifact_version": 1,
  "observation_version": 1,
  "set": { "name": "comemory-mixed-v1", "version": 1, "tasks": 12, "judgments": 31 },
  "environment": {
    "os": "macos", "arch": "aarch64", "cpus": 12,
    "peak_rss_bytes": null,
    "observation_bytes": 148233,
    "peak_task_observation_bytes": 21004,
    "working_set_repo": "comemory",
    "working_set_files": 4
  },
  "retrieval": { "binary_version": "0.35.1", "schema_version": "19",
                 "knobs": { "...": 0 }, "knobs_hash": "…",
                 "corpus": { "memories": 120, "code_symbols": 4310,
                             "documents": 18, "document_chunks": 96,
                             "repos": [ { "repo": "comemory",
                                          "last_head": "01fa67a6…",
                                          "last_indexed_at": "2026-09-18T12:00:00Z" } ],
                             "digest": "…" } },
  "reference_time": "2026-09-18T13:40:00Z",
  "decay_frozen": true,
  "budgets": { "min_tasks": 8, "min_ndcg_gain": 0.02,
               "max_ndcg_regression": 0.01, "max_p95_task_ms": 1500 },
  "arms": [
    { "name": "deterministic", "kind": "deterministic", "scorer_version": null,
      "scored_fraction": 1.0,
      "overall": { "tasks": 12, "judged_tasks": 12,
                   "pool_recall": 0.83, "pool_recall_ci": [0.61, 0.97],
                   "recall_at_k": 0.71, "recall_at_k_ci": [0.48, 0.90],
                   "mrr": 0.64, "mrr_ci": [0.42, 0.85],
                   "ndcg_at_k": 0.69, "ndcg_at_k_ci": [0.47, 0.88],
                   "judged_in_page": 29, "unjudged_in_page": 31,
                   "judged_page_fraction": 0.48 },
      "per_domain": { "memory": { "…": 0 }, "code": { "…": 0 }, "document": { "…": 0 } },
      "latency_ms": { "p50": 14, "p90": 31, "p95": 38, "max": 52, "mean": 18.4 },
      "paired_vs_baseline": null,
      "verdict": "baseline",
      "latency_within_budget": true }
  ],
  "tasks": [
    { "task_id": "mem-ranking-01",
      "observation": { "observation_version": 1, "query": "…", "filters": {},
                       "retrieval": {}, "reference_time": "…", "decay_frozen": true,
                       "pool_size": 200, "page_limit": 5, "page_offset": 0,
                       "candidates": [] },
      "page_matches_production": true,
      "judgments_matched": 1, "judgments_unmatched": 0, "judgments_stale": 0,
      "unmatched_targets": [],
      "arms": { "deterministic": { "pool_recall": 1.0, "recall_at_k": 1.0,
                                   "mrr": 1.0, "ndcg_at_k": 1.0,
                                   "judged_in_page": 1, "unjudged_in_page": 4 } } }
  ]
}
```

`paired_vs_baseline` on a non-baseline arm:

```json
{ "metric": "ndcg_at_k", "mean_delta": 0.041, "ci": [0.008, 0.077], "tasks": 12 }
```

### Metric definitions

**The unit of relevance is the judgment, not the candidate.** One judgment can
match more than one observation — a document judgment keyed on a relative path
matches every pooled candidate at that path, and a code judgment keyed on
`(repo, path, symbol)` matches every pooled candidate carrying that identity —
so counting candidates would inflate a denominator that is meant to say "how
many reviewed relevant things were there". Counting judgments also lets the
denominator include a relevant item retrieval never returned, which is exactly
what pool recall has to measure.

For one task, let:

- `Jl` = the task's judgments after stale ones (a version pin meeting a
  different content version) are removed;
- `Jp` = `{ j in Jl : j.relevance > 0 }`, the reviewed-relevant judgments;
- `P` = the candidate pool in `pool_position` order;
- `A` = the arm's ordering of `P`;
- `match(j)` = every observation in `P` whose `CandidateIdentity` satisfies `j`'s
  required keys, and its version pin when one is present.

Then:

- **`pool_recall`** = `|{ j in Jp : match(j) is non-empty }| / |Jp|`, and `0.0`
  when `Jp` is empty. Independent of the arm — every arm reorders the same
  pool — so this is retrieval's own ceiling for the task.
- **`recall_at_k`** = `|{ j in Jp : match(j) intersects the first k of A }| /
  |Jp|`, `0.0` on an empty `Jp`. The same convention as `metrics::recall_at_k`
  (an empty relevant set scores `0.0`, not `1.0`), expressed over judgments.
- **`mrr`** = `1 / r`, where `r` is the 1-based position of the first entry of
  `A` matched by any `j in Jp`; `0.0` when there is none —
  `metrics::first_hit_rank`'s convention.
- **`ndcg_at_k`** = `DCG@k / IDCG@k`. The gain at 1-based position `i` of `A` is
  `2^rel(i) - 1`, where `rel(i)` is the relevance of the highest-graded judgment
  in `Jl` matching that observation and `0` when none matches; the discount is
  `1 / log2(i + 1)`. `IDCG@k` takes the relevances of `Jp` sorted descending,
  truncated to `k`. `IDCG@k == 0` yields `0.0`.
- **Unjudged handling.** A page candidate no judgment matches contributes gain
  `0` and never enters a numerator. It is **not** the same as a judged-`0`
  candidate: the first is counted in `unjudged_in_page`, the second in
  `judged_in_page`, and `judged_page_fraction = judged_in_page / page_len`
  states the coverage outright, so a metric computed over a thinly judged page
  is visible rather than implied. Missing positives are never inserted into `P`.
- **Paired uncertainty.** For each non-baseline arm, the per-task delta
  `arm.ndcg_at_k - baseline.ndcg_at_k` is bootstrapped with
  `metrics::bootstrap_ci` (1000 resamples, the existing seeded `SplitMix64`
  stream) to a 95% percentile interval. A paired delta over identical candidate
  snapshots removes the corpus variance an unpaired comparison carries.
- **Verdict.** `Inconclusive` when `judged_tasks < budgets.min_tasks`;
  `Improved` when the delta CI's lower bound `> budgets.min_ndcg_gain`;
  `Regressed` when the CI's upper bound `< -budgets.max_ndcg_regression`;
  `Neutral` otherwise. `latency_within_budget` is reported separately, on
  `latency_ms.p95 <= budgets.max_p95_task_ms`, so a quality verdict and a
  latency verdict can never be confused for one another.
- **What `latency_ms` measures.** The wall clock of one task's pool retrieval
  (`run_legs` + `fuse_domains::fuse`) for the baseline arm, and that plus the
  arm's own ordering pass for a scored arm. The production-parity page query
  behind `page_matches_production` is issued separately and is **excluded** from
  every latency figure.

### Retrieval seam

```rust
// src/domains/retrieval/unified.rs

/// Everything one unified run is asked for, bundled so `find` and `run_legs`
/// take a query rather than seven positional arguments — the same reason
/// `Filters` and `DomainFilters` exist.
#[derive(Debug, Clone, Copy)]
pub struct UnifiedQuery<'a> {
    pub text: &'a str,
    pub vector: Option<&'a [f32]>,
    pub filters: Filters<'a>,
    pub domain_filters: DomainFilters<'a>,
}

/// The three legs' reranked rows for one run, before fusion discards their
/// passage text and version anchors.
pub struct LegRows {
    pub memory: Vec<rerank::Reranked>,
    pub code: Vec<code_rerank::CodeReranked>,
    pub documents: Vec<doc_route::DocHit>,
    /// The shared pool size every leg was fetched at.
    pub pool: usize,
}

/// Run every in-scope leg and return its own rows.
pub fn run_legs(
    cfg: &Config, conn: &Connection, query: UnifiedQuery<'_>, window: PageWindow,
) -> Result<LegRows>;

/// Fuse one run's legs into the ranked list, reading the memory hits' batched
/// navigation metadata on the way.
pub fn fuse_legs(
    cfg: &Config, conn: &Connection, legs: LegRows,
) -> Result<Vec<fuse_domains::UnifiedHit>>;

/// `find` is exactly `run_legs` + `fuse_legs` + `pipeline::paginate`; no leg, no
/// filter and no ordering differs, and the benchmark fuses through the same
/// `fuse_legs` rather than restating it.
pub fn find(
    cfg: &Config, conn: &Connection, query: UnifiedQuery<'_>, window: PageWindow,
) -> Result<UnifiedRun>;
```

`find`'s signature changes shape — seven positional arguments become a
`UnifiedQuery` plus the window — which is a Rust-level break for a library
caller. Its behavior, its output and every CLI and HTTP surface above it are
unchanged.

### Store seam

```rust
// src/store/code_text.rs

/// One `code_symbols` row's identity, version anchor and text.
pub struct CodeText {
    pub repo: String,
    pub path: String,
    pub symbol: String,
    pub blob_oid: String,
    pub line_start: i64,
    pub line_end: i64,
    pub snippet: String,
}

/// Batched read of `ids`, keyed by `code_symbols.id`. Ids with no live row
/// are absent from the map — a raced re-index delete is a missing entry, not
/// an error.
pub fn fetch(conn: &Connection, ids: &[i64]) -> Result<HashMap<i64, CodeText>>;

// src/store/documents.rs

/// Batched `revision_hash` read for `ids`, keyed by `documents.id`. The
/// document leg's counterpart to the code leg's read above: one query per run
/// rather than a point read per pooled document. Missing ids are absent.
pub fn fetch_revisions(conn: &Connection, ids: &[&str]) -> Result<HashMap<String, String>>;
```

Both are batched, and both exist only because fusion drops these fields: the
search path pays neither read.

## Failure modes and edge cases

| Case | Observable behavior |
| --- | --- |
| `--set` file missing or unreadable | `Error::Config` naming the path; exit 78 |
| Set YAML invalid, or `version` not `1` | `Error::Config` naming the file and the version; exit 78 |
| Two tasks share an `id` | Load error naming the duplicated id: task ids key the scores files and the per-task report, so a duplicate would silently merge two tasks |
| A judgment's `relevance` is above `3` | Load error naming the task id and the value; the graded scale is `0..=3` and an out-of-range grade would distort `2^rel - 1` |
| `budgets:` absent | Load error. Budgets must be declared in the reviewed dataset, before any arm is scored; inferring them after the fact is how a negative result becomes a positive one |
| A judgment target omits a required key, or carries a key belonging to another domain | Load error naming the task id, the domain and the key |
| `ranking:` block absent | Load error: the pinned configuration is required, never inherited from the live config |
| `ranking:` out of validated bounds | `Error::BadRequest` from `tune::validated_with_candidate`; nothing is scored or written |
| Task sets `kind` with `domain: code` | Load error naming the task id: `kind` narrows the memory leg only, so it would be silently inert |
| Task sets `lang` with `domain: memory`, or `path` with `domain: code` | Same shape of load error, same reason |
| `vectors:` declared but a task has no `vector` | Load error naming the task id |
| A task has a `vector` but `vectors:` is absent | Load error: lexical and BYO-vector are separate scenarios |
| A task's `vector` length differs from `vectors.dim` | Load error naming the task id and both lengths |
| Zero tasks after load | `Error::Unavailable`; exit 69 |
| A task's query returns zero candidates | Scored as a task with `pool_recall = 0`, kept in the denominator. Not an error — that is the measurement |
| A judgment matches no observation | Counted in `judgments_unmatched`, its target listed in `unmatched_targets` (the target's own keys — an unmatched judgment has no observed content version, so it has no `candidate_ref`). The candidate is **not** inserted |
| A judgment's version pin differs from the observation | Counted in `judgments_stale`, excluded from `R`, never treated as a match |
| A task has zero matched judgments | Excluded from `judged_tasks`; its metrics are `0.0` and it is reported, so coverage is visible |
| `judged_tasks < budgets.min_tasks` | Every arm's verdict is `Inconclusive`, regardless of the deltas |
| A code candidate's row vanished between fusion and the text read | The observation is emitted with empty `BoundedText` for that candidate and the run records it in `text_unavailable`; it is never dropped, because dropping would silently shrink the pool |
| A document candidate's parent row vanished | Same: emitted with an empty `revision_hash` recorded as `text_unavailable` |
| `--scores` file unreadable or invalid JSON | `Error::Config` naming the path; exit 78 |
| A scores file names a task id not in the set | Load error naming the id, so a stale scores file cannot be silently half-applied |
| A scores file covers a task only partially | Applied; `scored_fraction < 1.0` and the unscored candidates keep pool order |
| A scores file contains a score JSON cannot represent (`1e400`) | Parse error naming the scores file and the column. JSON has no NaN or infinity literal, so the parser refuses it before any arm is built |
| Two `--scores` files declare the same `arm` name | Load error naming the collision |
| A `--scores` file declares the arm name `deterministic` | Load error: that name is reserved for the baseline arm |
| `--report` path cannot be written | `Error::Io` naming the path, **after** the summary has been printed, so a measured run is not lost to a bad path |
| `peak_rss_bytes` unavailable (non-Linux) | `null`, documented; `observation_bytes` is always present |
| Corpus digest differs from a previous run | Not an error. The digest is recorded so a reader can see the corpus moved |

## Acceptance criteria

- **AC-1:** Running `comemory benchmark --set <mixed set> --report <path>` over a
  real indexed fixture corpus emits, for every candidate, a `candidate_ref`, a
  domain, a stable identity, a content version, `pool_position`, a nullable
  `returned_position`, and a `BoundedText` with a `sha256` of the untruncated
  text; and the artifact's envelope carries the query text, a complete
  `EffectiveFilters`, `RetrievalVersion` with `knobs_hash` and a
  `CorpusSnapshot` carrying `repo_marker.last_head` per repo, and
  `reference_time`.
- **AC-2:** `CandidateIdentity` round-trips through `candidate_ref` for all three
  domains, including a repo label, path or symbol containing `:` and `%`, and a
  malformed reference (unknown domain, wrong arity, bad escape, non-integer
  chunk ordinal) is a parse error naming the reference rather than an accepted
  candidate.
- **AC-3:** A set containing memory, code, document and `all` tasks scores every
  one of them against the real legs, and per-domain metrics are reported
  separately; a task that sets a filter belonging to another domain
  (`kind` on code, `lang` on memory, `path` on code) fails to load naming the
  task id.
- **AC-4:** For a task whose judged-relevant memory is in the candidate pool but
  outside the top `k`, `pool_recall` is `1.0` while `recall_at_k` is `0.0` in the
  same report, and the pool's candidate count is unchanged by the presence of a
  judgment that matched nothing.
- **AC-5:** The report states judgment coverage — `judged_in_page`,
  `unjudged_in_page`, `judged_page_fraction`, `judgments_matched`,
  `judgments_unmatched`, `judgments_stale` — and a judgment pinning a content
  version that no longer matches is counted in `judgments_stale` and scores as a
  miss rather than a hit.
- **AC-6:** Two arms (the deterministic baseline and one from a `--scores` file)
  are scored over byte-identical candidate observations for every task, and the
  non-baseline arm reports a paired `mean_delta` with a 95% bootstrap interval
  plus per-domain metrics.
- **AC-7:** A benchmark run writes no `retrieval_log` row and bumps no
  `memories.access_count` or `code_symbols.access_count`, and two runs of the same
  set over an unchanged corpus with `ranking.decay: 0.0` produce identical
  `recall_at_k`, `mrr` and `ndcg_at_k`, with `decay_frozen: true` recorded.
- **AC-8:** A set declaring `vectors: { model, dim }` requires every task to
  supply a vector of exactly `dim` floats — a missing or wrong-length vector is a
  load error naming the task — and the artifact records
  `VectorScenario::Supplied { model, dim, digest }`; a set without `vectors`
  records `Lexical` and rejects any task carrying a vector.
- **AC-9:** The artifact records dataset size (task and judgment counts, corpus
  row counts), a latency distribution (`p50`/`p90`/`p95`/`max`/`mean`),
  hardware (`os`, `arch`, `cpus`), memory use (`observation_bytes`,
  `peak_task_observation_bytes`, and `peak_rss_bytes` where the platform exposes
  it) and the run-scoped working set (`working_set_repo`, `working_set_files`),
  and a set whose judged tasks fall below `budgets.min_tasks` yields verdict
  `Inconclusive` for every arm even when the point deltas are positive.
- **AC-10:** `comemory eval` is unchanged: the same golden YAML file produces the
  same `EvalReport` JSON field set and the same `recall_at_k` / `mrr` values
  before and after this change; and for one query and filter set, the first `k`
  entries of the benchmark's own candidate pool carry the same ids, in the same
  order, as `comemory find`'s page — proving the benchmark measures the
  shipped ranking rather than a second composition of the legs.
- **AC-11:** The artifact written by `--report` is replayable: parsing it back
  yields every `candidate_ref` and bounded text needed to build a `--scores`
  file, and feeding that file back as an arm scores without re-reading the
  corpus for candidate identity.

## Acceptance evidence

| AC | Real input / fixture | Expected observable result | Boundary / failure case | Runnable check |
| --- | --- | --- | --- | --- |
| AC-1 | Real temp data dir: memories saved through the binary, a real git repo indexed with `index-code`, a real markdown tree indexed with `index`; `tests/common/fixtures/benchmark/mixed-v1.yaml` | Artifact JSON contains all listed fields, non-empty `corpus.repos[0].last_head` | Corpus with zero code rows for a code task → `pool_recall: 0`, not an error | `tests/cli__benchmark.rs::benchmark_emits_a_full_observation_artifact` |
| AC-2 | Identities with `repo = "a:b"`, `path = "src/a%b.rs"`, `symbol = "m::f"` | `parse(encode(x)) == x` for all three domains | `"memory:only-one"`, `"video:x:y"`, `"document:a:b:zz"`, `"code:a:b%ZZ:c:d"` all error naming the ref | `src/domains/learning/evaluation/tests/candidate_identity.rs` |
| AC-3 | The mixed set's four tasks over the same fixture corpus | `per_domain` carries `memory`, `code`, `document` blocks with distinct task counts | Set file with `domain: code` + `filters.kind` → load error naming `code-rerank-01` | `src/domains/learning/evaluation/tests/benchmark_set.rs`, `tests/cli__benchmark.rs::benchmark_scores_every_domain` |
| AC-4 | A task whose relevant memory is deliberately ranked below `k = 1` in the fixture corpus | Same report shows `pool_recall = 1.0`, `recall_at_k = 0.0` | A judgment targeting an id absent from the corpus → `judgments_unmatched = 1`, `candidates.len()` unchanged | `src/domains/learning/evaluation/tests/benchmark_runner.rs` |
| AC-5 | Mixed set with one judgment pinning a stale `content_hash` | `judgments_stale = 1`, that task's `recall_at_k = 0.0`, coverage fields present | Task with zero matched judgments → excluded from `judged_tasks`, still listed | `src/domains/learning/evaluation/tests/benchmark_runner.rs` |
| AC-6 | A scores file built from the run's own artifact that reverses the pool order | Both arms present; observations byte-identical between arms; `paired_vs_baseline.ci` present and the delta negative | Scores file naming an unknown task id → load error; partial coverage → `scored_fraction < 1.0` | `tests/cli__benchmark.rs::benchmark_compares_two_arms_over_one_snapshot` |
| AC-7 | Run the same set twice against an unchanged corpus | Identical metrics; `SELECT COUNT(*) FROM retrieval_log` is `0`; `SUM(access_count)` unchanged | `ranking.decay: 0.5` → `decay_frozen: false` recorded | `tests/cli__benchmark.rs::benchmark_is_repeatable_and_writes_no_telemetry` |
| AC-8 | A set with `vectors: { model: "test:unit", dim: 1024 }` and a deterministic 1024-float vector per task, over memories saved with matching vectors | `filters.vector` is `Supplied` with a stable `digest` | A task missing `vector`, a 3-float vector, and a `vector` without `vectors:` all load-error naming the task | `src/domains/learning/evaluation/tests/benchmark_set.rs`, `tests/cli__benchmark.rs::benchmark_runs_a_supplied_vector_scenario` |
| AC-9 | The mixed set; then the same set with `budgets.min_tasks: 99` | Environment, latency and size blocks populated; second run's every arm verdict is `"inconclusive"` | Non-Linux → `peak_rss_bytes: null`, `observation_bytes` still present | `tests/cli__benchmark.rs::benchmark_reports_budgets_and_environment` |
| AC-10 | The existing `tests/cli__eval.rs` golden fixtures, unchanged; then the same fixture corpus queried through both surfaces | `comemory eval --golden … --json` field set and values unchanged; benchmark pool prefix ids `==` `find` page ids | A query whose production pool (`clamp(2k, 50, max_window)`) is narrower than the benchmark pool records `page_matches_production: false` without failing the run | `cargo nextest run --all-features` (whole suite), plus `src/domains/learning/evaluation/tests/benchmark_runner.rs::pool_prefix_matches_find_page` |
| AC-11 | The artifact from AC-1 | Re-parsed artifact yields every `candidate_ref`; an arm built from it scores | Truncated artifact file → parse error naming the path | `tests/cli__benchmark.rs::benchmark_artifact_round_trips_into_an_arm` |

## Documentation impact

- `docs/designs/2026-09-18-domain-aware-retrieval-benchmark.md` — this document;
  the `## Candidate observation contract` section is what #209 / #210 / #211 /
  #213 / #214 consume.
- `docs/cli-reference.md` — regenerated by `bash scripts/regen-cli-docs.sh` for
  the new subcommand.
- `docs/scenarios/benchmark.md` — new per-command scenario file; every flag gets
  a scenario citing a real test, enforced by `tests/cli_scenario_catalog.rs`.
- `README.md` — one entry in the command list and a short "Offline benchmark"
  paragraph pointing at the design doc.
- `AGENTS.md` — `comemory benchmark` in Key Commands; the
  `domains/learning/` module-map row gains the benchmark files.
- `src/domains/learning/evaluation/README.md` — six new rows.
- `src/store/README.md` — one new row for `code_text.rs`.
- `src/domains/retrieval/README.md` — `unified.rs`'s row notes `run_legs`.
- No new environment variable, no config key, no migration.

## Open Questions

1. **Should the benchmark record a run row in `eval_runs`?** Resolved: no.
   `eval_runs.kind` has a `CHECK (kind IN ('eval','tune','bandit'))`, so a
   `benchmark` row needs a migration, and the console's recall sparkline reads
   that table with memory-only semantics. A benchmark row would corrupt both.
   Non-blocking; #209 owns persistence.
2. **Should the code leg's working-set affinity prior be pinned?**
   `code_search::search_code_hits` builds it from the process CWD
   (`WorkingSet::from_cwd`), which is machine-dependent, and it does not hand
   the resolved set back to its caller. Resolved for this issue: it is **not**
   pinned — pinning it would change a production code path, which Non-Goal 3
   forbids. Instead the benchmark calls the already-public
   `WorkingSet::from_cwd(None)` **once**, at run start, purely to report it, and
   records `environment.working_set_repo` and `environment.working_set_files`.
   That is run-scoped and indicative, not a per-task echo of what each code leg
   actually used — the field docs say exactly that — but it makes the one
   machine-dependent input to code ranking visible in the artifact instead of
   invisible. Owner: this issue; the per-task exactness is a follow-up note on
   #213, which already has to touch this path. Non-blocking.
3. **Is `peak_rss_bytes` worth a system-introspection dependency?** Resolved:
   no. Reading `/proc/self/status`'s `VmHWM` costs twenty lines and no
   dependency on Linux (which is what CI runs); every other platform reports
   `null` and the portable `observation_bytes` proxy. Non-blocking.
