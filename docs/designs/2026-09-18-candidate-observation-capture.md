# Candidate observation capture and version-bound judgments — Design

**Date:** 2026-09-18   **Status:** Approved   **Author:** comemory maintainers
**Topic:** Persist the #208 candidate observation contract from a real query, and
record typed relevance judgments against what was actually observed.

## Problem

`retrieval_log` records a query, a JSON list of returned ids, two filters and an
elapsed time. It is not a candidate observation: it carries no candidate text,
no content version, no pool order, and — for `comemory find` — only the *memory*
ids of a run that returned three domains. `domains::retrieval::find::track_run`
filters the fused page down to `DOMAIN_MEMORY` before writing the row, so a code
or document hit leaves no trace at all.

Three consequences follow, and all three are the reason #209 exists.

1. **History cannot be rejoined.** `feedback_events` rows carry text-encoded
   `code_symbols` rowids. Re-indexing purges and reinserts a touched file's rows
   and SQLite recycles the freed rowids, so an old rowid may name a different
   symbol today. `domains::learning::code_feedback` already keys its counters on
   the stable `(repo, path, symbol)` triple for exactly this reason and refuses
   to join historical rowids to today's symbols. Any attempt to reconstruct a
   training example from `retrieval_log` + `feedback_events` would silently
   re-attribute a verdict to whatever symbol inherited the number.
2. **The passage is discarded before anyone can hold it.**
   `unified::fuse_domains::fuse` flattens each leg's row into a `UnifiedHit`
   carrying `title`, `subtitle`, `repo`, `path` and a score. The memory body, the
   code snippet and the winning document chunk — the text retrieval actually
   matched on — are dropped at that boundary. Reconstructing them later means
   reading whatever the file says *now*, which is a different document.
3. **Two of the three domains cannot be judged at all.** `comemory feedback`
   accepts memory ids and `code_symbols` rowids. There is no path for a document
   verdict, and the code path is keyed on the recyclable rowid.

#208 specified the contract that fixes this — `docs/designs/2026-09-18-domain-aware-retrieval-benchmark.md`
§ *Candidate observation contract* — and built it for the offline benchmark,
which holds observations in memory for the duration of one `comemory benchmark`
run and writes them to an operator-named artifact file. Nothing persists an
observation produced by a real query, and nothing resolves a human verdict
against one. This design adds both.

## Non-Goals

1. **No dataset export.** Turning stored observations and judgments into a
   training file is #210. This design defines the schema #210 reads; it writes
   no dataset format.
2. **No reranking on the search path, and no configuration on a search surface.**
   #213 owns that. `comemory find` gains no ranking behaviour here.
3. **No model, no Python, no training.** #212 and #214.
4. **No change to the #208 contract.** `OBSERVATION_VERSION` stays `1`. No
   field is added, removed, renamed or given new semantics. Two #208 *functions*
   are generalized (§ Architecture), which is an implementation change, not a
   contract change.
5. **No change to the #211 wire protocol.**
6. **No change to `comemory feedback`, `feedback`, `code_feedback`,
   `feedback_events` or `retrieval_log`.** The legacy counters and both HTTP
   feedback routes keep their exact behaviour, provenance vocabulary and JSON
   shape. Judgments are a second, parallel record, not a replacement.
7. **No backfill.** No historical `retrieval_log` or `feedback_events` row is
   ever turned into an observation. There is no code path that could.
8. **No capture outside `comemory find` / `GET|POST /api/v1/find`.** `search`,
   `search-code` and `context` are single-domain and keep their existing
   telemetry unchanged.
9. **No cloud sync.** Observations and judgments are local-only.
10. **No new `eval_runs` rows and no new `eval_runs.kind` value.** That table's
    `CHECK` is `kind IN ('eval','tune','bandit')` and the console sparkline
    reads it with memory-only semantics.

## Architecture

### The shape

```text
comemory find / GET|POST /api/v1/find
  |
  +-- unified::run_legs ................ LegRows: memory / code / document rows,
  |                                      passage text and version anchors intact
  |
  +-- candidate_facts::collect ......... identity + content version + BoundedText
  |   (only when capture is armed)       READ HERE, while identity is known
  |
  +-- unified::fuse_legs ............... the fused pool (UnifiedHit; text gone)
  |
  +-- observation_capture::capture ..... QueryObservation -> two tables
  |   (best effort; a failure warns)     opt-in, bounded, never fails the search
  |
  +-- pipeline::paginate ............... the page the caller sees, unchanged
```

`find::run` already owns the telemetry decision (`track`), so it is the one
place that can arm capture without a second retrieval path. It currently calls
`unified::find`, which is documented as being exactly `run_legs` + `fuse_legs`
+ `paginate`. Capture needs the leg rows *before* fusion and the fused pool
*before* pagination, so `find::run` composes those three published steps
itself — **unconditionally**, whether or not capture is armed, so there is one
retrieval path through `find` and not two. Only the two capture calls between
them are conditional. `unified::find` is unchanged and remains the single-call
entry point for `benchmark_runner::page_matches` and for library callers; its
doc gains one line saying why `retrieval::find::run` composes the steps rather
than calling it.

`comemory judge` then resolves a typed verdict against a stored observation:

```text
comemory judge --observation o-… --ref <candidate_ref>=<relevance> …
  |
  +-- candidate_identity::parse_ref .... refuse a malformed / unknown-domain ref
  +-- JudgmentTarget::resolve .......... #208's own per-domain key validation
  +-- TargetKey::matches ............... Yes / Stale / No against each observed
  |                                      CandidateIdentity — never a locator
  +-- all-or-nothing write ............. one row per matched target
```

### Decisive trade-offs

**Capture is armed by `observations.enabled && track`, not by `enabled` alone.**
`track` is already `false` on a read-only `comemory serve` (`routes::track_for`)
and under the `COMEMORY_DISABLE_ACCESS_TRACKING` test hook. Capture is a write;
tying it to `track` means `GET /api/v1/find` on a read-only server writes
nothing without a second gate to keep in sync, and `/find` keeps its
`mutating: false` route-table entry honestly. The cost is that a deliberately
untracked run cannot be captured — which is correct, because such a run has no
`retrieval_log` row for its `query_id` either.

**Capture failure is best-effort and never propagates.** Exactly like
`pipeline::log_retrieval`, which already returns `Option<String>` and warns
rather than failing a search. A `find` whose capture write fails returns its
hits, exits `0`, and reports `observation_id: null`. This is proven by dropping
the table out from under a live database, not asserted (AC-9).

**A judgment is bound to one observation and addressed by `candidate_ref`.**
The alternative — a free-floating judgment matched against every observation by
stable identity — cannot satisfy the contract's first rule, because nothing
would stop a reviewer inserting a positive retrieval never returned. Binding to
an `observation_id` makes that structurally impossible: a target that matches no
candidate in *that* pool is refused and named as a pool-recall miss.
`candidate_ref` is #211's opaque domain-qualified token, so the storage key, the
CLI argument and the reranker protocol's candidate id are one string.

**The locator is stored as opaque JSON with no index on it.** `title`, `path`,
`line_range`, `heading_path` and `symbol_id` live in one `locator_json` column.
No column, index or predicate anywhere in this design reads them, so rule 2
("never match on a locator") is enforced by the absence of a way to do it. The
document *path* that a judgment does match on is `DocumentIdentity::path`, which
is inside `candidate_ref`.

**Purge redacts, it does not delete.** When `gc` hard-deletes a trashed memory,
every candidate observation naming that memory has its `text` blanked and its
`unresolved` flag set; the row, its pool position and its ref stay. Deleting the
row would punch a hole in the recorded pool and silently corrupt pool recall;
keeping the body would resurrect content the user deleted. Redaction is the only
option that does neither. This is the one place this design departs from
`store::memory_purge`'s stated "`retrieval_log` stays" rule, and for a stated
reason: a `retrieval_log` row holds ids, a candidate observation holds the body.

**Retention keeps judged observations.** `gc` evicts observations past
`prune.learning_retention_days` **only when no judgment references them**. A
reviewed judgment is the expensive artifact; evicting the observation it was
made against would leave a verdict with nothing to verify it, and rule 4 would
have no content version to compare.

### What is reused, and the two generalizations

Reused unchanged: `candidate_identity` (`CandidateIdentity`, `parse_ref`,
`candidate_ref`), `candidate_observation` (`OBSERVATION_VERSION`, `BoundedText`,
`CandidateLocator`, `CandidateObservation`, `QueryObservation`,
`EffectiveFilters`, `VectorScenario`, `RetrievalVersion`, `canonical_digest`),
`judgment` (`MAX_RELEVANCE`, `Judgment`, `JudgmentTarget`, `TargetKey`,
`MatchOutcome`), `candidate_facts::collect`, `run_environment::retrieval_version`,
`unified::{run_legs, fuse_legs, UnifiedQuery, LegRows}`, `store::code_text::fetch`,
`store::documents::fetch_revisions`, `utilities::dated_id`,
`utilities::telemetry` (the `manual` / `implicit` provenance vocabulary and the
`source::FIND` const), `pipeline::{pool_size, paginate}`.

Two #208 helper functions are generalized so this design reuses them instead of
restating them. Neither changes a contract field:

| Function | Was | Becomes | Why |
| --- | --- | --- | --- |
| `benchmark_observe::observe` | `observe(pool, facts, k: usize)` | `observe(pool, facts, window: PageWindow)` | A benchmark run is always `offset = 0`; a real `find` is not. `QueryObservation::page_offset` already exists in the contract and is documented as "`0` for every benchmark run", so non-zero offsets were always anticipated. The new rule matches `pipeline::paginate` exactly. `benchmark_runner` passes `PageWindow { offset: 0, limit: k }`. |
| `benchmark_observe::effective_filters` | `effective_filters(task, set, time_scope)` | `effective_filters(inputs: FilterInputs<'_>, time_scope, vector: VectorScenario)` | The mapping from "the filters that were in effect" onto `EffectiveFilters` is identical for a benchmark task and a `find` request; only the source of the values differs. The three-line task-to-`FilterInputs` mapping moves to `benchmark_runner`, its only caller. |

### Ownership and the declared capability edge

| Concern | Home |
| --- | --- |
| The two observation tables and the judgment table, and every SQL statement over them | `src/store/candidate_observations.rs`, `src/store/candidate_judgments.rs`, declared in `src/store/schema_learning.rs` |
| Building a `QueryObservation` from a real run and writing it best-effort | `src/domains/learning/observation_capture.rs` |
| Resolving typed judgments against a stored observation | `src/domains/learning/judge.rs` |
| clap flags, TTY rendering, exit codes | `src/cli/judge.rs` |

Capture is a learning signal recorded by a retrieval run, which makes
`find::run` a caller of `domains::learning`. That adds one directed edge to
`scripts/architecture-policy.json`'s `owner_dependencies`:
`domains::retrieval -> domains::learning`. It is the same shape as the existing
`domains::graph -> domains::learning` edge, which exists so
`graph::materialize` can mint a co-activation reward through
`learning::feedback_tracking::record_implicit_used`. The reverse edge
(`domains::learning -> domains::retrieval`) already exists, as do
`domains::retrieval -> domains::graph` and
`domains::graph -> domains::learning`, so this introduces no new class of edge.

## Interfaces / Schema

### Config: the `[observations]` section

Named `observations`, not `capture`: `domains::capture` is the unrelated
coding-session capture capability and `[capture]` would read as its section.

| Key | Type | Default | Env | Meaning |
| --- | --- | --- | --- | --- |
| `observations.enabled` | bool | `false` | `COMEMORY_OBSERVATIONS_ENABLED` | Opt in to query-time candidate capture. Off, nothing is read, built or written, and `find` behaves exactly as today. |
| `observations.max_text_bytes` | usize | `4096` | `COMEMORY_OBSERVATIONS_MAX_TEXT_BYTES` | Per-candidate text bound handed to `BoundedText::bound`. Validated `> 0`. The same default the benchmark set declares. |
| `observations.max_candidates` | usize | `100` | `COMEMORY_OBSERVATIONS_MAX_CANDIDATES` | Ceiling on persisted candidates per captured query. Validated `> 0`. The pool is cut at this many, never below the returned page. |

Retention is **not** a new key: observations age out on the existing
`prune.learning_retention_days` (`COMEMORY_LEARNING_RETENTION_DAYS`, default
90), the same window `retrieval_log` and `feedback_events` use.

### Table `candidate_query_observations`

One row per captured query. `#[table]` in `src/store/schema_learning.rs`.

| Column | Type | Null | Meaning |
| --- | --- | --- | --- |
| `observation_id` | TEXT | PK | `o-<yyyymmdd>-<8hex>`, minted by `utilities::dated_id` |
| `observation_version` | INTEGER | NOT NULL | `candidate_observation::OBSERVATION_VERSION` at write time |
| `query_id` | TEXT | NULL | The `retrieval_log.query_id` of the same run; NULL when the log write failed |
| `query` | TEXT | NOT NULL | Query text, verbatim |
| `source` | TEXT | NOT NULL | `utilities::telemetry::source` const; `find` today |
| `filters_json` | TEXT | NOT NULL | `EffectiveFilters` as JSON |
| `retrieval_json` | TEXT | NOT NULL | `RetrievalVersion` as JSON |
| `knobs_hash` | TEXT | NOT NULL | `RetrievalVersion::knobs_hash`, denormalized |
| `corpus_digest` | TEXT | NOT NULL | `RetrievalVersion::corpus.digest`, denormalized |
| `decay_frozen` | INTEGER | NOT NULL | `1` when `knobs.decay == 0.0` |
| `pool_size` | INTEGER | NOT NULL | `LegRows::pool` — the window every leg was fetched at |
| `page_limit` | INTEGER | NOT NULL | `PageWindow::limit` |
| `page_offset` | INTEGER | NOT NULL | `PageWindow::offset` |
| `candidate_count` | INTEGER | NOT NULL | Rows actually written to `candidate_observations` |
| `truncated` | INTEGER | NOT NULL | `1` when `max_candidates` cut the pool; a consumer must not compute pool recall from a truncated observation |
| `at` | TEXT | NOT NULL | `store::memory_row::iso_format` of the run's start. This is the contract's `reference_time` **and** the GC key: the format is fixed-width ISO-8601 UTC, so a plain string `<` is chronological, exactly as `retrieval_log.at` relies on |

Index: `idx_candidate_query_observations_at` on `(at)`.

### Table `candidate_observations`

One row per observed candidate. `#[primary_key(observation_id, pool_position)]`.

| Column | Type | Null | Meaning |
| --- | --- | --- | --- |
| `observation_id` | TEXT | NOT NULL | Parent |
| `pool_position` | INTEGER | NOT NULL | 1-based, retrieval's own order |
| `domain` | TEXT | NOT NULL | `CHECK (domain IN ('memory','code','document'))` |
| `candidate_ref` | TEXT | NOT NULL | #211's opaque token; `parse_ref` inverts it to the full `CandidateIdentity` |
| `content_version` | TEXT | NOT NULL | `CandidateIdentity::content_version`, denormalized; empty string when unresolved |
| `unresolved` | INTEGER | NOT NULL | `1` when no content snapshot exists for this candidate — its row vanished mid-run, or a purge redacted it. Never matchable by a judgment |
| `returned_position` | INTEGER | NULL | 1-based position on the displayed page; NULL when the candidate was in the pool but below the cut |
| `retrieval_score` | REAL | NOT NULL | `UnifiedHit::score` |
| `rank_in_domain` | INTEGER | NOT NULL | 1-based position in the candidate's own leg, before fusion |
| `tier` | INTEGER | NULL | Memory lexical ladder tier; NULL for code and document |
| `text` | TEXT | NOT NULL | `BoundedText::text` |
| `text_sha256` | TEXT | NOT NULL | `BoundedText::sha256` — digest of the FULL text, before bounding |
| `text_full_bytes` | INTEGER | NOT NULL | `BoundedText::full_bytes` |
| `text_truncated` | INTEGER | NOT NULL | `BoundedText::truncated` |
| `locator_json` | TEXT | NOT NULL | `CandidateLocator` as JSON. Display only; nothing indexes or predicates on it |

Index: `idx_candidate_observations_ref` on `(candidate_ref)`.

### Table `candidate_judgments`

One reviewed verdict per target per observation.
`#[primary_key(observation_id, candidate_ref)]`.

| Column | Type | Null | Meaning |
| --- | --- | --- | --- |
| `observation_id` | TEXT | NOT NULL | The captured query this verdict answers |
| `candidate_ref` | TEXT | NOT NULL | The candidate that was judged, as observed |
| `domain` | TEXT | NOT NULL | `CHECK (domain IN ('memory','code','document'))` |
| `relevance` | INTEGER | NOT NULL | `CHECK (relevance >= 0 AND relevance <= 3)` — `judgment::MAX_RELEVANCE` |
| `provenance` | TEXT | NOT NULL | `CHECK (provenance IN ('manual','implicit'))`; `utilities::telemetry::{PROV_MANUAL, PROV_IMPLICIT}` |
| `at` | TEXT | NOT NULL | `iso_format` |

A re-judgment of the same `(observation_id, candidate_ref)` replaces the row.

### Migration

`migrations/0020_candidate_observations.sql`, generated by
`just migration candidate_observations`, journalled, `Class::Additive`,
markers `["0020_candidate_observations"]`, wired into
`store::migrate::list::MIGRATIONS`; `CURRENT_VERSION` becomes `"20"`.
Each of the three table names is added to `store::schema::DECLARED_TABLES` and
`store::schema::registry()`.

### Rebuild, purge, GC and sync

| Lifecycle | Treatment |
| --- | --- |
| **Rebuild** (`comemory rebuild`) | All three tables join `maintenance::rebuild::copy::COPIED_TABLES` and are copied by `store::rebuild_copy_learning_events::copy_event_and_mined_tables`. Markdown cannot reconstruct any of them; dropping them would destroy every reviewed judgment. |
| **Purge** (`gc` hard-deleting a trashed memory, `store::memory_purge::purge_memory`) | Every `candidate_observations` row whose `candidate_ref` is under the `memory:<id>:` prefix has `text` set to `''`, `text_full_bytes` to `0`, `text_truncated` to `0` and `unresolved` to `1`, inside the same transaction as the rest of the purge. The row, its `pool_position`, its `candidate_ref` and its score survive, so the recorded pool keeps its shape. Judgment rows are untouched and are now judgments of an unresolved candidate, which a consumer must skip. |
| **GC retention** (`comemory gc`) | `candidate_query_observations` rows with `at < cutoff` **and no `candidate_judgments` row** are deleted along with their `candidate_observations` children. A judged observation is retained indefinitely. `gc`'s response gains `observation_rows`. |
| **Sync** | Never pushed and never pulled. Neither table appears in any `domains::sync` projection, manifest or import path. Observations hold local query text and local passages; the code-index push is deliberately snippet-free and this is the same rule. |
| **Repo drop / re-index** | Untouched. Re-indexing a repo changes `blob_oid`, which makes a later observation a different content version — that is what the contract's staleness rule is for, not a deletion. |
| **`comemory unindex` / `DELETE /sources`** | Untouched, deliberately, and this is the one place the purge rule does *not* generalize. Unindexing removes comemory's index of a source root; the operator's file is still on disk and was never comemory's to delete. A purged memory is the opposite case: `gc` has unlinked the markdown, so the observation would be the last surviving copy of a body the user deleted. Deletion of user content redacts; withdrawal of an index does not. |

### `comemory find`

`--json` gains one key and the TTY view gains one trailing line, both only when
capture produced an observation:

```json
{ "hits": [...], "query_id": "q-20260918-1a2b3c4d",
  "observation_id": "o-20260918-9f8e7d6c",
  "limit": 12, "offset": 0, "has_more": true, "total": 40 }
```

`observation_id` is `null` when capture is disabled, suppressed (`track` false)
or failed. `GET|POST /api/v1/find` carries the same key. No flag is added to
`find`; the capture decision is config, per the #213 non-goal.

### `comemory judge`

CLI-only (`serve::routes::meta::CLI_ONLY`), like `benchmark`: a verdict against
a locally captured observation is a local review action, and the HTTP feedback
routes keep their own unchanged contract.

```
comemory judge --observation <ID> [--ref <CANDIDATE_REF>=<RELEVANCE>]...
               [--source explicit|implicit] [--json]
```

| Flag | Default | Effect |
| --- | --- | --- |
| `--observation` | required | The `o-<yyyymmdd>-<8hex>` id printed by `comemory find` |
| `--ref` | unset, repeatable | `<candidate_ref>=<relevance>`; relevance `0..=3` |
| `--source` | `explicit` | `explicit` stores provenance `manual`, `implicit` stores `implicit` — the #130 vocabulary, through `learning::feedback_tracking::Source` |

With no `--ref`, `judge` writes nothing and reports the observation: its query,
its corpus digest, and one line per candidate (pool position, returned position,
domain, `candidate_ref`, title, any recorded relevance).

`--json` shape when recording:

```json
{ "observation_id": "o-20260918-9f8e7d6c", "recorded": 3, "provenance": "manual" }
```

`--json` shape when reporting:

```json
{ "observation_id": "o-20260918-9f8e7d6c", "query": "frontmatter contract",
  "observation_version": 1, "knobs_hash": "…", "corpus_digest": "…",
  "truncated": false,
  "candidates": [ { "pool_position": 1, "returned_position": 1,
                    "domain": "memory", "candidate_ref": "memory:5a9f19bc:…",
                    "title": "…", "unresolved": false, "relevance": 3 } ] }
```

### Rust surface

```rust
// src/domains/learning/observation_capture.rs

/// Whether this run may capture: opt-in config AND a run allowed to write
/// telemetry. Cheap enough to call before any leg is fetched.
pub fn armed(cfg: &Config, track: bool) -> bool;

/// Everything one captured run supplies that retrieval does not already hold.
pub struct CaptureInput<'a> {
    pub query: &'a str,
    pub query_id: Option<&'a str>,
    pub source: &'static str,
    pub filters: EffectiveFilters,
    pub window: PageWindow,
    pub pool: &'a [fuse_domains::UnifiedHit],
    pub facts: &'a FactsByHit,
}

/// Build the `QueryObservation` and persist it. Returns the new
/// `observation_id`. Every error is the caller's to swallow — see `record`.
pub fn capture(cfg: &Config, conn: &Connection, input: CaptureInput<'_>) -> Result<String>;

/// `capture`, best effort: a failure is warned through `tracing` and returns
/// `None`, so an optional capture can never fail a search.
pub fn record(cfg: &Config, conn: &Connection, input: CaptureInput<'_>) -> Option<String>;

/// The `EffectiveFilters` one `find` run applied.
pub fn find_filters(
    filters: Filters<'_>, domain_filters: DomainFilters<'_>,
    vector: Option<&[f32]>, embed_model: &str,
) -> EffectiveFilters;

// src/domains/learning/judge.rs

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub observation: String,
    #[serde(default)] pub refs: Vec<String>,
    #[serde(default)] pub source: Option<String>,
}

pub enum Outcome { Recorded(Recorded), Report(Box<Report>) }
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Outcome>;

/// The `JudgmentTarget` an observed identity addresses — the inverse of a
/// reviewed set's hand-written target, pinned to the version observed.
/// #210 uses it to render a benchmark set from stored judgments.
pub fn target_of(identity: &CandidateIdentity) -> JudgmentTarget;

// src/store/candidate_observations.rs

pub struct NewObservation<'a> { /* the 15 header columns */ }
pub struct NewCandidate<'a> { /* the 15 candidate columns */ }
pub struct StoredCandidate { /* read-back row, plus the judged relevance */ }
/// One unit of work: the header and every candidate, inside an internal
/// `unchecked_transaction` — the caller holds `&Connection`, exactly as
/// `pipeline::record_telemetry` does.
pub fn insert(conn: &Connection, header: &NewObservation<'_>, candidates: &[NewCandidate<'_>]) -> Result<()>;
pub fn fetch_header(conn: &Connection, observation_id: &str) -> Result<Option<StoredObservation>>;
pub fn fetch_candidates(conn: &Connection, observation_id: &str) -> Result<Vec<StoredCandidate>>;
pub fn redact_memory(conn: &Connection, memory_id: &str) -> Result<u64>;
pub fn evict_unjudged_before(conn: &Connection, cutoff: &str) -> Result<(u64, u64)>;

// src/store/candidate_judgments.rs

pub struct NewJudgment<'a> { /* 6 columns */ }
pub fn upsert_all(conn: &Connection, rows: &[NewJudgment<'_>]) -> Result<u64>;
pub fn fetch_for_observation(conn: &Connection, observation_id: &str) -> Result<HashMap<String, i64>>;
```

## Failure modes and edge cases

| Case | Observable behaviour |
| --- | --- |
| `observations.enabled = false` | No facts collected, no rows written, no extra query run. `find --json` reports `observation_id: null`. `find`'s cost is byte-identical to today. |
| `track = false` (read-only server, or `COMEMORY_DISABLE_ACCESS_TRACKING`) | Capture is not armed. Nothing is written. `observation_id: null`. |
| Capture write fails (table missing, disk full, database locked) | `tracing::warn!` names the `observation_id` it was minting and the error; `find` returns its hits and exits `0` with `observation_id: null`. |
| `candidate_facts::collect` fails | Same: the whole capture is one best-effort unit. |
| A candidate's row vanished between fusion and the facts read | `candidate_facts` already gives it `BoundedText::unavailable` and an empty content version. The row is written with `unresolved = 1`, keeping its pool position. It can never be judged. |
| Pool larger than `max_candidates` | The first `max_candidates` positions are written, `truncated = 1`, `candidate_count` is what was written. When the returned page is longer than `max_candidates`, the cut is raised to cover the whole page — a returned candidate is never dropped. |
| Empty pool (no hit in any leg) | A header row is written with `candidate_count = 0`. A query that returned nothing is a real observation and a real pool-recall miss for every judgment. |
| `--observation` is not a `o-<yyyymmdd>-<8hex>` id | `Error::Config` naming the value and the expected shape, exit `78`, before the database is opened. |
| `--observation` names no stored observation | `Error::Unavailable` naming the id (`unavailable: observation `o-…` not found`), exit `69`; nothing written. `Error::NotFound` is deliberately not used: it maps to exit `64` and its Display is `memory not found:`, which would misname the subject. |
| Stored `observation_version` is not `OBSERVATION_VERSION` | Refused with `Error::Config` naming the id and both versions. Never read further, never guessed at. |
| `--ref` is malformed / unknown domain / wrong arity / non-integer ordinal | `candidate_identity::parse_ref`'s own error, naming the reference. Exit `78`. Nothing written. |
| `--ref` missing `=`, or a relevance above `MAX_RELEVANCE`, or not an integer | `Error::Config` naming the reference and the accepted range. Exit `78`. Nothing written. |
| A `--ref` matches no candidate in that observation | Refused: `Error::Config` naming every such reference as a candidate-pool recall miss. **Nothing is written, including the refs that did match.** Rule 1. |
| A `--ref` whose identity matches but whose content version differs | Refused as **stale**, named separately from the misses, with both versions. Nothing written. Rule 4. |
| A `--ref` matching an `unresolved` candidate | Refused, named as unresolvable: there is no content snapshot to judge. |
| The same `--ref` given twice | Last value wins within the call; one row is written. Deterministic because the refs are processed in argument order. |
| `--source` is neither `explicit` nor `implicit` | `Error::BadRequest` from `feedback_tracking::Source::parse`, the existing #130 behaviour. |
| Judged observation reaches the retention window | Retained. Only unjudged observations are evicted. |
| Memory purged after being judged | The candidate row is redacted (`unresolved = 1`, empty text); the judgment survives and now describes an unresolvable candidate. |
| Re-index recycles a `code_symbols` rowid | Irrelevant: no stored column, index or predicate in this design contains a `code_symbols` rowid. `locator_json.symbol_id` carries it for display and is never read back for matching. |
| `comemory rebuild` | All three tables are copied; judgments and observations survive. |
| Two concurrent captures | Each mints its own `o-` id from a nanosecond-seeded digest and writes its own rows in one transaction. There is no shared mutable row. |

## Acceptance criteria

- **AC-1:** With `observations.enabled = true`, `comemory find` over a corpus
  holding real memories, a real indexed git repo and a real indexed markdown
  tree writes exactly one `candidate_query_observations` row and one
  `candidate_observations` row per pooled candidate, and the observation
  contains candidates from all three domains. Each row's `candidate_ref` parses
  back through `candidate_identity::parse_ref` to the identity it was built
  from.
- **AC-2:** Every captured candidate carries the text retrieval matched on — a
  memory's body, a code symbol's snippet, a document chunk's passage — with
  `text_sha256` equal to the SHA-256 of the **full** text and `text_full_bytes`
  its untruncated length, including when `max_text_bytes` truncated `text`.
  Code candidates carry a non-empty `blob_oid` inside their ref; document
  candidates carry a non-empty `revision_hash` and their winning
  `chunk_ordinal`; memory candidates carry the body digest that equals
  `memories.content_hash`.
- **AC-3:** A captured observation distinguishes the pool from the page:
  candidates below the page cut have `returned_position` NULL while keeping
  ascending `pool_position`, and with `--offset` set the `returned_position`
  values match the page `find` actually printed. `pool_size`, `page_limit` and
  `page_offset` record the window.
- **AC-4:** The observation preserves the complete effective filters, the
  retrieval and corpus versions and the reference time: `filters_json` names
  exactly the domains in scope and the `repo` / `kind` / `lang` / `path` /
  `since` / `until` / `as-of` values that were applied, and `retrieval_json`'s
  `knobs_hash` and `corpus.digest` equal what
  `run_environment::retrieval_version` reports for the same config and corpus.
- **AC-5:** With `observations.enabled = false` (the default), a `comemory find`
  over the same corpus leaves both observation tables empty and reports
  `observation_id: null`.
- **AC-6:** `comemory judge --observation <id> --ref <ref>=<n>` records one
  `candidate_judgments` row per reference, with provenance `manual` by default
  and `implicit` under `--source implicit`, and `comemory feedback`'s counters,
  `feedback_events` rows and JSON output are unchanged by it.
- **AC-7:** A `--ref` naming a target that is not in that observation's pool is
  refused, named as a pool-recall miss, and **no** row from that call is
  written — including the references that did match.
- **AC-8:** After a memory's body changes and is re-saved under a new id, a
  `--ref` built from the *old* observation is refused as stale against a *new*
  observation, naming both content versions; and after the old memory is
  deleted, the same ref is refused as a pool-recall miss. Neither writes a row.
- **AC-9:** With capture enabled but `candidate_observations` dropped out of the
  live database, `comemory find` still returns its ranked hits, exits `0`, and
  reports `observation_id: null`.
- **AC-10:** Re-indexing a git repository after editing an indexed file —
  which purges and reinserts that file's `code_symbols` rows, freeing rowids
  SQLite may recycle — leaves the stored record addressing the same
  `(repo, path, symbol)` it was captured for. Specifically: no
  `candidate_observations` or `candidate_judgments` column contains a
  `code_symbols` rowid (the rowid appears only inside `locator_json`); a code
  judgment recorded before the re-index still names its symbol afterwards; and
  a ref carrying the pre-edit `blob_oid` is refused as **stale** against a
  post-re-index observation of the same symbol. The test asserts these
  invariants rather than asserting that SQLite in fact recycled a particular
  number, which is not a reproducible outcome.
- **AC-11:** Replacing a document's content and re-running `comemory index`
  changes the document's `revision_hash`, so a ref built from the pre-edit
  observation is refused as stale against the post-edit one; the pre-edit
  observation and its judgment still hold the passage that was actually shown.
- **AC-12:** `comemory gc` deletes an unjudged observation past
  `prune.learning_retention_days` together with its candidate rows and reports
  the count, and retains a judged observation of the same age.
- **AC-13:** `comemory gc` hard-deleting a trashed memory redacts every
  candidate observation of that memory — empty `text`, `unresolved = 1` — while
  the row, its `pool_position` and its `candidate_ref` remain, and a later
  `judge --ref` against that candidate is refused as unresolvable.
- **AC-14:** `comemory rebuild` preserves every observation, candidate and
  judgment row, and the live-table coverage test still accounts for every table
  exactly once.
- **AC-15:** A stored observation whose `observation_version` is not
  `OBSERVATION_VERSION` is refused by `judge`, naming the id and both versions,
  and nothing is read from its candidates.
- **AC-16:** Delayed feedback resolves against the corpus **as it was
  observed**, not as it is now: after a capture, the corpus is changed (a new
  memory saved and an indexed file edited and re-indexed), and a judgment is
  only then recorded against the original observation. It resolves to the
  original candidate and stores the content version that was observed, while
  the same ref against a fresh observation of the changed corpus is refused as
  stale. A verdict arriving late is therefore never silently re-attributed to
  today's content.
- **AC-17:** `comemory judge --observation <id>` with no `--ref` writes nothing
  and reports the observation's candidates, their pool and page positions, and
  any relevance already recorded.

## Acceptance evidence

| AC | Real input / fixture | Expected observable result | Boundary / failure case | Runnable check |
| --- | --- | --- | --- | --- |
| AC-1 | Three memories through `comemory save`, a real git repo through `index-code --repo demo`, a markdown tree through `index` | One header row; one candidate row per pooled hit; all three `domain` values present; every ref round-trips | Empty pool writes a header with `candidate_count = 0` | `tests/cli__judge.rs::capture_records_every_domain_in_one_observation` |
| AC-2 | A memory whose body exceeds `max_text_bytes`; a real Rust symbol; a markdown chunk | `text_sha256` equals the digest of the full body; `text_full_bytes` > `length(text)`; `text_truncated = 1` | A code row deleted between fusion and the facts read yields `unresolved = 1` | `tests/cli__judge.rs::captured_text_is_bounded_and_digested_before_truncation`, `src/domains/learning/tests/observation_capture.rs` |
| AC-3 | The same corpus, `find --k 2` then `find --k 2 --offset 2` | Page-one candidates 1..2 carry `returned_position` 1..2, the rest NULL; page two's carry 1..2 for pool positions 3..4 | `--offset` beyond the pool writes every `returned_position` NULL | `tests/cli__judge.rs::an_observation_separates_the_pool_from_the_page` |
| AC-4 | `find --repo demo --kind decision --lang rust --path '**/*.md' --since …` | `filters_json` carries each value under the right key and lists exactly the in-scope domains; `knobs_hash` equals `run_environment::retrieval_version`'s | A filter left unset is `null`, not omitted | `tests/cli__judge.rs::an_observation_records_the_complete_effective_filters` |
| AC-5 | Default config | Both tables empty; `observation_id` null | — | `tests/cli__judge.rs::capture_is_off_by_default` |
| AC-6 | One captured observation, three refs across three domains | Three judgment rows, provenance `manual`; `--source implicit` stores `implicit`; `feedback`'s own tables untouched | `--source bogus` exits with the existing bad-request error | `tests/cli__judge.rs::judgments_record_across_every_domain_under_the_stated_provenance` |
| AC-7 | A ref for a memory that exists but was not in the pool, alongside a ref that was | Exit `78`, message naming the missing ref as a pool-recall miss; `candidate_judgments` empty | Every ref missing behaves the same | `tests/cli__judge.rs::a_judgment_retrieval_never_returned_is_refused_and_nothing_is_written` |
| AC-8 | Capture, edit the memory body, re-save, capture again | Old ref stale against the new observation, naming both hashes; after `delete`, the same ref is a pool-recall miss | The unchanged ref still matches | `tests/cli__judge.rs::a_changed_memory_makes_a_pinned_judgment_stale`, `…::a_deleted_memory_is_a_pool_recall_miss` |
| AC-9 | `DROP TABLE candidate_observations` on the live database, then `find` | Exit `0`, hits printed, `observation_id` null, warning on stderr | The header table dropped instead behaves the same | `tests/cli__judge.rs::a_failing_capture_never_fails_the_search` |
| AC-10 | A git repo indexed, a tracked file edited and committed, `index-code` re-run | Stored refs unchanged in `(repo, path, symbol)`; no stored identity column holds a rowid; pre-edit `blob_oid` ref refused as stale | A symbol deleted by the edit becomes a pool-recall miss, not a mismatch | `tests/cli__judge.rs::reindexing_that_reuses_code_rowids_never_moves_a_judgment` |
| AC-11 | A markdown file re-written and re-indexed | `revision_hash` differs; the old ref is stale; the old observation still holds the old passage | A chunk that moved ordinal is a different ref | `tests/cli__judge.rs::a_replaced_document_chunk_makes_the_old_ref_stale` |
| AC-12 | Two observations aged past the window by a direct `at` update, one judged | `gc` deletes the unjudged pair and reports it; the judged one and its candidates remain | A window of `0` days evicts everything unjudged | `tests/cli__judge.rs::gc_evicts_unjudged_observations_and_keeps_judged_ones` |
| AC-13 | Capture a memory hit, `delete` it, `gc` with a zero trash window | Candidate row present with empty `text` and `unresolved = 1`; `judge --ref` on it refused | The other candidates in the same observation are untouched | `tests/cli__judge.rs::purging_a_memory_redacts_its_captured_text_and_keeps_the_pool_shape` |
| AC-14 | Capture, judge, `comemory rebuild` | Every row survives; `migration_integrity_every_live_table_is_covered_exactly_once` green | A table missing from both allowlists fails the coverage test | `tests/coverage.rs` (existing), `tests/cli__judge.rs::rebuild_preserves_observations_and_judgments` |
| AC-15 | An observation row whose `observation_version` is set to `999` | Exit `78`, message naming the id, `999` and `1` | — | `tests/cli__judge.rs::an_unknown_observation_version_is_refused` |
| AC-16 | Capture, then save a new memory and re-index an edited file, then `judge` the original observation | The judgment resolves against the observed version; the same ref is stale against a fresh observation | The changed candidate's fresh ref judges normally | `tests/cli__judge.rs::a_delayed_verdict_resolves_against_the_corpus_it_observed` |
| AC-17 | One captured observation, one recorded judgment | The report lists every candidate with its positions and the recorded relevance; no row is written | An observation with no judgments reports `relevance: null` | `tests/cli__judge.rs::judge_without_a_verdict_reports_the_observation` |

## Documentation impact

- `docs/designs/2026-09-18-candidate-observation-capture.md` — this file.
- `docs/scenarios/judge.md` — new, one scenario per flag, cited tests.
- `docs/scenarios/find.md` — the new `observation_id` key and the TTY line.
- `docs/scenarios/gc.md` — the new `observation_rows` count and the
  judged-observation retention rule.
- `docs/cli-reference.md` — regenerated by `bash scripts/regen-cli-docs.sh`.
- `docs/guides/schema-migrations.md` — the declared-table count.
- `migrations/README.md` — the `0020` row.
- `AGENTS.md` — the `[observations]` section's three env vars in the
  Environment Variables table, the three tables in the Architecture bullet, the
  new `comemory judge` command in Key Commands, and the learning-capability and
  store rows in the Module Map.
- `README.md` — `comemory judge` in the command list.
- `src/store/README.md`, `src/domains/learning/README.md`, `src/cli/README.md` —
  one row per new file.
- `docs/designs/2026-09-17-domain-first-migration-inventory.md` — one file-ledger
  row per new production file.
- `scripts/architecture-policy.json` — the
  `domains::retrieval -> domains::learning` owner dependency.
- `tests/api__parity.rs` — its own CLI-only exception list gains `judge`,
  alongside `serve::routes::meta::CLI_ONLY`.
- `src/domains/learning/evaluation/README.md` — no row changes; the two
  generalized `benchmark_observe` functions keep their file.

## Open Questions

1. **Should `search` / `search-code` / `context` capture too?** Resolved, no —
   Non-Goal 8. They are single-domain, their hit shapes already carry their own
   ids, and the hazard this issue names is specific to `find`. A later issue may
   widen it; the schema's `source` column is already there for that.
   Non-blocking.
2. **Should a judgment be allowed to float free of an observation?** Resolved,
   no — it is the only structural guarantee of rule 1. #210 exports reviewed
   sets whose targets *are* version-free, which is where a free-floating target
   belongs. Non-blocking.
3. **Does the code leg's CWD-derived working-set prior belong in the captured
   filters?** Resolved, no — #208 records it in `RunEnvironment` for a benchmark
   run and explicitly leaves pinning it to #213. This design does not touch it.
   Non-blocking.
