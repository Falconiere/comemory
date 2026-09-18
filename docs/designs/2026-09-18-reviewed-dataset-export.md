# Reviewed relevance dataset export — Design

**Date:** 2026-09-18   **Status:** Approved   **Author:** comemory maintainers
**Topic:** Turn stored candidate observations and reviewed judgments into a
versioned, leakage-checked JSONL dataset plus a manifest, reproducible byte for
byte from one database snapshot.

## Problem

#209 persists what retrieval actually produced: `candidate_query_observations`
holds one row per captured query with its effective filters, its ranking
configuration digest and its corpus snapshot digest; `candidate_observations`
holds every pooled candidate with its domain-qualified identity, the content
version retrieval saw and the bounded passage it matched on;
`candidate_judgments` holds one reviewed relevance verdict per candidate per
observation. Nothing reads those three tables together.

A trainer cannot consume them as they stand, and the ways of getting it wrong
are the reason this design exists rather than a `SELECT ... JOIN`.

1. **The existing feedback surfaces mix evidence classes.** `feedback_events`
   carries both `manual` and `implicit` provenance (#130), and
   `domains::learning::evaluation::golden::harvest` pins `PROV_MANUAL`
   precisely because HTTP-implicit rows carry real query ids and would
   otherwise become evaluation ground truth. An export that treats the two
   alike silently promotes a pseudo-label to truth. Access frequency and the
   absence of a click are not negative relevance judgments and must not become
   one here either.
2. **A candidate nobody judged is not a negative.** A pool of one hundred
   candidates with three verdicts has three labels and ninety-seven unknowns.
   Emitting the ninety-seven as zeros manufactures training signal out of a
   reviewer's silence. A reviewed relevance of `0` — "I looked at this and it
   is not relevant" — is a different thing, and is exactly the hard negative a
   reranker needs.
3. **An unresolved candidate has no content to train on.** `unresolved` is set
   when a candidate's row vanished mid-capture and when `store::memory_purge`
   redacted a purged memory's passage. The row, its pool position and its score
   survive so the recorded pool keeps its shape, but the text is gone. Such a
   candidate can never be judged and can never be exported as labeled data; it
   can only be reported.
4. **Near-duplicate queries, adjacent passages and repeated content versions
   leak across a random split.** Two observations of "frontmatter contract" and
   "Frontmatter, contract?" are the same question. Two chunks of one document,
   two symbols in one file, and two content versions of one memory are the same
   content. A row-level random split puts one on each side and reports a
   quality number that measures memorization.
5. **A held-out qualification set that lives in the same directory as the
   training data is not held out.** #214 will mine negatives and select a
   checkpoint from whatever the export wrote; the export is the only place that
   can make the qualification rows structurally unavailable at that moment.

## Non-Goals

1. **No change to the #208 contract, the #209 schema or the #211 wire
   protocol.** `OBSERVATION_VERSION` stays `1`, the three tables keep their
   columns, and `candidate_ref` keeps its encoding. This design reads them.
2. **No model, no framework, no Python.** The export is JSONL and JSON on the
   local filesystem. Training is #214's and lives outside Rust.
3. **No training recipe.** Loss functions, batching, pair construction and
   checkpoint selection are #214's. This design hands that work a dataset and
   a manifest, and states what each field means.
4. **No new retrieval behavior, no reranking, no search-surface configuration.**
   Nothing under `src/domains/retrieval/`, `src/serve/` or `src/config/`
   changes; those are #213's.
5. **No new capture path.** The export reads what `comemory find` already
   captured under `[observations] enabled` and what `comemory judge` already
   recorded. It never runs retrieval and never writes to the three tables.
6. **No HTTP route.** The command writes to an operator-named filesystem path,
   which is what already keeps `benchmark` and `judge` CLI-only.
7. **No implicit-judgment writer.** `candidate_judgments.provenance` admits
   `implicit`, and `judge::Request::source` is the seam that would write one,
   but nothing on the CLI does. This design defines how an implicit label is
   exported if one ever exists; it does not create one.
8. **No pseudo-labelling, no weak supervision, no negative mining from
   unjudged rows.** The export never invents a label. Its only "mining" step is
   a deterministic cap on how many reviewed negatives one query contributes.

## Architecture

### The shape

One new CLI-only command core reads the three tables through one new store
module, runs a fixed nine-step deterministic pipeline, and writes a directory
of JSONL files plus one manifest.

```text
candidate_query_observations ─┐
candidate_observations       ─┼─> store::candidate_dataset::snapshot
candidate_judgments          ─┘    (three reads, one read transaction)
        │
        v
  dataset_rows      refuse unknown contract version
                    drop unresolved candidates (count them)
                    apply the domain filter
                    collapse duplicate observations
                    classify every judgment: matched / stale / recall miss
                    resolve contradictory judgments within a provenance class
        │
        v
  dataset_split     query group key + content group key per row
                    union-find over the bipartite (query, content) graph
                    one group_id per connected component
                    forced holdout (repo / time slice) then seeded hash
        │
        v
  bucket by (split, provenance class)
        │
        v
  negative selection, per bucket only
        │
        v
  <out>/train.jsonl  <out>/validation.jsonl  [<out>/holdout.jsonl]
  [<out>/<split>.implicit.jsonl]             <out>/manifest.json
```

### Decisive trade-offs

**Splits are assigned to connected components, not to rows or to queries.** A
row links one query group to one content group. Grouping by query alone lets a
document that two different queries both retrieved appear in train and in
holdout; grouping by content alone lets the same question appear on both sides.
Union-find over the bipartite graph is the only assignment that closes both,
and it is cheap: the graph has one node per distinct query group plus one per
distinct content group, and one edge per exported row. The cost is that a
sufficiently connected corpus collapses into one giant component and therefore
one split. That is the honest outcome — a corpus whose queries all share
content cannot be split without leakage — and the manifest reports the
component sizes so the operator can see it rather than discovering it as an
implausible evaluation score.

**The content group key drops the content version and the intra-file
position.** A memory's key is `memory:<memory_id>`, a code candidate's is
`code:<repo>:<path>`, a document's is `document:<path>`. Two content versions of
one memory, two symbols in one file and two chunks of one document therefore
share a group by construction, which is what "prevent near-duplicate queries,
adjacent document chunks and repeated content versions from crossing splits"
asks for. Dropping the symbol from the code key is deliberate: two symbols in
one source file share imports, identifiers and prose, and are the code analogue
of adjacent chunks.

**The query group key is a sorted, deduplicated bag of lowercased alphanumeric
tokens.** "frontmatter contract", "Frontmatter, contract?" and "contract
frontmatter" hash to one key. This is a coarse near-duplicate rule chosen for
being total, deterministic and explicable rather than for being the best
clustering available. `utilities::simhash` was the alternative; it needs a
radius, and a radius makes group membership depend on which other queries
happen to be present, which makes a split assignment unstable as the corpus
grows. Bag-of-words identity is stable: adding a query never moves an existing
one between splits unless it joins two components, and the manifest records
every group so a merge is visible.

**Split assignment is a seeded hash of the group id, not a shuffle.** A shuffle
needs the whole population, so every new observation reshuffles every existing
one. `sha256(seed || group_id)` mapped into `[0, 1)` and compared against the
cumulative ratios is stable under growth: an existing group keeps its split
forever unless the seed or the ratios change. The price is that the realized
ratios only approach the requested ones; the manifest reports the realized
counts, never the requested ones, as the truth.

**The holdout split is withheld by default and its digest is published.**
Without `--include-holdout` the command writes no `holdout.jsonl` at all, so
training-time mining and model selection have no file to read. The manifest
still carries the holdout's row count and the SHA-256 the file would have had,
so a later qualification run can prove it scored the same rows. Publishing a
digest leaks nothing; publishing the rows would defeat the split. The command
also deletes every file in its own closed owned-name set from `--out` before
writing, so a `holdout.jsonl` left by an earlier `--include-holdout` run cannot
survive into a run that withholds it.

**The export never reads `locator_json`.** #209 gives the locator no index
specifically so that "never match a judgment on a display field" is structural
rather than aspirational. This design extends that one step: the store helper
does not select the column, so a title, a path shown on screen or a
`code_symbols` rowid cannot reach a training record at all. Identity, content
version and passage text are what a record carries.

**The export never runs retrieval.** Every number in a record was measured at
capture time and is read back verbatim. A record therefore describes the corpus
the observation saw, not today's, which is the whole point of #209 storing a
content version.

**The three reads are one read transaction.** A capture landing between two
separate statements would hand the export a header with no candidates, or
candidates with no header, and would make "the same snapshot" mean nothing.
`store::candidate_dataset::snapshot` opens one `unchecked_transaction`, runs
all three reads inside it, and returns owned rows, so the export reads one
consistent view of the three tables even while `comemory find` is capturing.

### Where the code lives

| File | Responsibility |
| --- | --- |
| `src/store/candidate_dataset.rs` | The three window reads and their owned row types. All SQL. Never selects `locator_json` |
| `src/domains/learning/evaluation/dataset_record.rs` | `RECORD_VERSION`, `DatasetRecord` and its parts; the JSONL line shape |
| `src/domains/learning/evaluation/dataset_rows.rs` | Contract-version refusal, unresolved handling, the domain filter, judgment classification, row building, `RowStats` |
| `src/domains/learning/evaluation/dataset_dedup.rs` | Duplicate-observation collapse and contradictory-judgment resolution |
| `src/domains/learning/evaluation/dataset_split.rs` | Group keys, union-find, forced holdout, seeded assignment, `SplitPlan` |
| `src/domains/learning/evaluation/dataset_manifest.rs` | `DatasetManifest` and its build |
| `src/domains/learning/dataset_export.rs` | The command core: `Request`, `run`, the owned-file sweep, the deterministic writes |
| `src/cli/export_dataset.rs` | The clap surface, the TTY summary and the `--json` acknowledgement |

Every one of those files is subject to the 300-code-line ceiling with `///`
lines counted, and `dataset_record.rs` carries a twenty-five-field documented
struct plus five smaller types. If it measures over, its label and revision
parts split into `dataset_label.rs` beside it rather than losing documentation;
the same applies to `dataset_export.rs`, whose write half splits into
`dataset_files.rs`.

Reused without change: `evaluation::candidate_identity::{CandidateIdentity,
CandidateDomain, parse_ref}`, `evaluation::candidate_observation::{
OBSERVATION_VERSION, EffectiveFilters, RetrievalVersion}`,
`evaluation::judgment::MAX_RELEVANCE`, `utilities::telemetry::{PROV_MANUAL,
PROV_IMPLICIT}`, `utilities::digest::sha256_hex`, `utilities::context::Ctx`,
`store::candidate_observations` (unchanged; the export adds a sibling rather
than widening its projections, because `fetch` is a per-observation read for
`judge` and this is a bulk windowed read).

## Interfaces / Schema

### `comemory export-dataset`

CLI-only (`serve::routes::meta::CLI_ONLY`), like `benchmark`: the run writes a
directory of files to an operator-named path.

```
comemory export-dataset --out <DIR>
    [--provenance manual|implicit|all]
    [--include-unjudged] [--include-holdout]
    [--domain memory|code|document]...
    [--since <WHEN>] [--until <WHEN>]
    [--split <TRAIN,VALIDATION,HOLDOUT>]
    [--split-seed <STRING>]
    [--holdout-repo <LABEL>] [--holdout-since <WHEN>]
    [--max-negatives-per-query <N>]
    [--json]
```

| Flag | Default | Effect |
| --- | --- | --- |
| `--out` | required | Output directory, created if absent. Every file in the command's own owned-name set is removed from it before writing; nothing else in the directory is touched |
| `--provenance` | `manual` | Which judgment provenance reaches the dataset. `manual` exports reviewed verdicts only. `implicit` exports implicit ones only. `all` exports both, into separate files |
| `--include-unjudged` | off | Also emit a record for every retrieved candidate with no verdict, carrying `"label": null`. Off by default, so the dataset is reviewed labels only |
| `--include-holdout` | off | Also write the holdout split's files. Off by default: the qualification rows are withheld and only their counts and digests appear in the manifest |
| `--domain` | all three | Restrict records to one or more domains. Repeatable. Applied before grouping |
| `--since` / `--until` | unset | Half-open window `[since, until)` on `candidate_query_observations.at`, parsed by `utilities::when` |
| `--split` | `0.7,0.15,0.15` | Train, validation and holdout ratios. Three finite values, each `>= 0`, summing to `1.0` within `1e-9` |
| `--split-seed` | `comemory-dataset-v1` | Salt for the group-id hash. Changing it reassigns every group |
| `--holdout-repo` | unset | Force every group that contains a `code` candidate in this repo into the holdout split, whatever its hash says |
| `--holdout-since` | unset | Force every group that contains an observation captured at or after this instant into the holdout split |
| `--max-negatives-per-query` | `0` (unlimited) | Cap how many reviewed relevance-`0` records one query group contributes to one split, keeping the lowest pool positions |

Exit behavior follows the crate's existing mapping: an invalid ratio, an
unknown provenance word or an unparsable date is `Error::Usage` /
`Error::Config`; a filesystem failure is `Error::Io`.

### The JSONL record

One line per `(observation_id, candidate_ref)` pair. `RECORD_VERSION` is `1`
and is carried on every line, so #214 refuses a shape it does not know exactly
as a consumer must refuse an unknown `observation_version`.

```jsonc
{
  "record_version": 1,
  "observation_version": 1,
  "split": "train",                       // "train" | "validation" | "holdout"
  "group_id": "g-1a2b3c4d5e6f7081",       // the connected component this row belongs to
  "query_group": "qg-9f8e7d6c5b4a3928",   // bag-of-words key for the query
  "content_group": "code:comemory:src/domains/retrieval/rerank.rs",
  "observation_id": "o-20260918-9f8e7d6c",
  "query_id": "q-20260918-1a2b3c4d",      // null when the retrieval_log write failed
  "query": "frontmatter contract",
  "source": "find",
  "reference_time": "2026-09-18T20:07:43Z",
  "domain": "code",                       // "memory" | "code" | "document"
  "candidate_ref": "code:comemory:src/domains/retrieval/rerank.rs:rerank:9d1c1f0b",
  "identity": {                           // CandidateIdentity, serde-tagged on "domain"
    "domain": "code",
    "repo": "comemory",
    "path": "src/domains/retrieval/rerank.rs",
    "symbol": "rerank",
    "blob_oid": "9d1c1f0b"
  },
  "content_version": "9d1c1f0b",
  "text": "pub fn rerank(...) { ... }",
  "text_sha256": "4c2f...",               // digest of the FULL passage, before bounding
  "text_full_bytes": 4821,
  "text_truncated": true,
  "label": {                              // null when the candidate was not judged
    "relevance": 3,                       // 0..=3; 0 is a reviewed hard negative
    "provenance": "manual",               // "manual" | "implicit"
    "at": "2026-09-18T20:11:02Z"
  },
  "pool_position": 4,                     // 1-based, retrieval's own order
  "returned_position": 3,                 // 1-based page position, null below the cut
  "retrieval_score": 0.8123,
  "rank_in_domain": 2,
  "tier": null,                           // memory lexical ladder tier, null elsewhere
  "pool_truncated": false,                // the observation's `truncated`; true forbids pool recall
  "decay_frozen": true,
  "filters": { /* EffectiveFilters, verbatim from filters_json */ },
  "retrieval_revision": {
    "binary_version": "0.35.0",
    "schema_version": "20",
    "knobs_hash": "7b2a9c...",
    "corpus_digest": "0f1e2d..."
  }
}
```

`retrieval_revision` is the four-field summary; the full `RetrievalVersion` —
every ranking knob and the whole corpus snapshot — appears once per distinct
`(knobs_hash, corpus_digest)` pair in the manifest's `retrieval_revisions`, so
a record stays small and nothing is lost.

`identity` is `CandidateIdentity`'s own serialization, which is the contract's
and not a second spelling of it. `candidate_ref` is the only matchable key;
`identity` is the parsed form of that same key, carried so a trainer need not
implement the codec.

There is no locator, no title and no `code_symbols` rowid anywhere in a record.

### The manifest

`<out>/manifest.json`, pretty-printed, written last. `MANIFEST_VERSION` is `1`.
It carries no wall-clock field: a timestamp would make two exports of one
snapshot differ, and reproducibility is worth more than a creation date that
`reference_time` already implies per observation.

```jsonc
{
  "manifest_version": 1,
  "record_version": 1,
  "observation_version": 1,
  "dataset_id": "d-3c1f5a90b7e24d68",
  "snapshot_digest": "a91c...",           // sha256 over the admitted input rows
  "binary_version": "0.35.0",
  "schema_version": "20",

  "filters": {
    "provenance": "manual",
    "include_unjudged": false,
    "include_holdout": false,
    "domains": ["code", "document", "memory"],
    "since": null,
    "until": null,
    "max_negatives_per_query": 0
  },

  "split": {
    "policy": "grouped-hash",
    "seed": "comemory-dataset-v1",
    "ratios": { "train": 0.7, "validation": 0.15, "holdout": 0.15 },
    "holdout_repo": null,
    "holdout_since": null,
    "groups": 12,
    "largest_group_rows": 31,
    "assignments": [
      { "group_id": "g-1a2b...", "split": "train", "forced": null,
        "queries": 2, "contents": 5, "rows": 9 }
    ]
  },

  "counts": {
    "observations_scanned": 40,
    "observations_refused_version": 0,
    "observations_truncated": 2,
    "duplicate_observations_dropped": 3,
    "candidates_scanned": 1200,
    "candidates_unresolved": 7,
    "judgments_on_unresolved_candidates": 2,
    "candidates_filtered_by_domain": 0,
    "judgments_scanned": 310,
    "judgments_stale": 1,
    "judgments_recall_miss": 0,
    "judgments_contradictory_dropped": 2,
    "candidates_unjudged": 900,
    "negatives_capped": 0,
    "rows_emitted": 300
  },

  "by_domain":  { "code": 90, "document": 60, "memory": 150 },
  "by_split":   { "holdout": 45, "train": 210, "validation": 45 },
  "by_label": [
    { "labels": "implicit", "relevance": 2,    "rows": 0   },
    { "labels": "manual",   "relevance": 0,    "rows": 40  },
    { "labels": "manual",   "relevance": 3,    "rows": 60  },
    { "labels": "none",     "relevance": null, "rows": 150 }
  ],

  "retrieval_revisions": [
    { "knobs_hash": "7b2a9c...", "corpus_digest": "0f1e2d...",
      "binary_version": "0.35.0", "schema_version": "20",
      "knobs": { /* RetrievalKnobs */ }, "corpus": { /* CorpusSnapshot */ } }
  ],

  "files": [
    { "path": "train.jsonl", "split": "train", "labels": "manual",
      "rows": 210, "bytes": 412233, "sha256": "5e11..." }
  ],

  "withheld": [
    { "path": "holdout.jsonl", "split": "holdout", "labels": "manual",
      "rows": 45, "bytes": 88120, "sha256": "c740...",
      "reason": "holdout withheld from this export; re-run with --include-holdout" }
  ]
}
```

`by_domain` and `by_split` are `BTreeMap`s, so their keys serialize in sorted
order and the manifest is byte-stable. `by_label` is a sorted sequence rather
than a map because a label has two dimensions — its evidence class and its
grade — and a single-keyed map cannot state both without ambiguity: an implicit
judgment of relevance `2` and a reviewed one are different rows and must count
separately. `assignments`, `files`, `withheld`, `by_label` and
`retrieval_revisions` are sequences sorted by their leading fields.

`snapshot_digest` is `sha256` over, in order, every admitted observation's
`(observation_id, at, observation_version, knobs_hash, corpus_digest,
candidate_count, truncated)`, every admitted candidate's `(observation_id,
pool_position, candidate_ref, content_version, unresolved, text_sha256)` and
every loaded judgment's `(observation_id, candidate_ref, relevance, provenance,
at)`, each field newline-joined. The candidate rows are in the digest because a
purge redacts a passage in place — blanking `text` and setting `unresolved` —
without touching its observation header, so a digest over headers alone would
report two materially different inputs as the same snapshot. Two exports whose
`snapshot_digest` agrees read the same input rows.

`dataset_id` is `d-` plus the first sixteen hex characters of `sha256` over
`RECORD_VERSION`, the serialized `filters` object, the serialized `split`
configuration (policy, seed, ratios, forced-holdout keys) and
`snapshot_digest`. Two runs with the same id produce byte-identical files.

### Output files

| Name | Written when | Holds |
| --- | --- | --- |
| `train.jsonl` | always | train rows with a `manual` label, plus train unjudged rows under `--include-unjudged` |
| `validation.jsonl` | always | the same for validation |
| `holdout.jsonl` | `--include-holdout` | the same for holdout |
| `train.implicit.jsonl` | `--provenance implicit` or `all` | train rows with an `implicit` label, and nothing else |
| `validation.implicit.jsonl` | `--provenance implicit` or `all` | the same for validation |
| `holdout.implicit.jsonl` | both of the above | the same for holdout |
| `manifest.json` | always | the manifest |

Those seven names are the command's owned-name set. Every one of them is
removed from `--out` before anything is written, whether or not this run will
write it.

Rows inside a file are ordered by `(group_id, observation_id, pool_position)`,
which is a total order because `pool_position` is unique within an observation.
Each line is `serde_json::to_string` of one `DatasetRecord` — struct field
order, no maps — followed by `\n`.

### Rust surface

```rust
// src/store/candidate_dataset.rs

/// One `candidate_query_observations` row, every column the export reads.
/// `pool_size`, `page_limit` and `page_offset` are deliberately absent: no
/// record carries them, and a projection that reads a column nobody consumes
/// is dead weight on every row.
pub struct DatasetObservation { /* 13 header columns */ }
/// One `candidate_observations` row. `locator_json` is deliberately absent.
pub struct DatasetCandidate { /* 14 columns, no locator */ }
/// One `candidate_judgments` row.
pub struct DatasetJudgment { /* 6 columns */ }

/// The three tables read as one consistent view.
pub struct DatasetSnapshot {
    /// Observations whose `at` falls in the half-open window, ascending by
    /// `(at, observation_id)`.
    pub observations: Vec<DatasetObservation>,
    /// Their candidates, ascending by `(observation_id, pool_position)`.
    pub candidates: Vec<DatasetCandidate>,
    /// Their judgments, ascending by `(observation_id, candidate_ref)`.
    pub judgments: Vec<DatasetJudgment>,
}

/// Read all three tables for the half-open `at` window `[since, until)` inside
/// ONE read transaction, so a capture landing mid-read cannot hand the caller a
/// header without its candidates.
pub fn snapshot(conn: &Connection, since: Option<&str>, until: Option<&str>)
    -> Result<DatasetSnapshot>;

// src/domains/learning/evaluation/dataset_record.rs

/// Version of the exported record shape. A consumer that reads an unknown
/// value must refuse the record.
pub const RECORD_VERSION: u32 = 1;
/// Which split a record belongs to.
pub enum Split { Train, Validation, Holdout }
/// Which evidence class a file carries.
pub enum LabelClass { Manual, Implicit }
/// One exported example.
pub struct DatasetRecord { /* the shape above */ }
/// The verdict on one record, absent when nobody judged the candidate.
pub struct RecordLabel { pub relevance: u8, pub provenance: String, pub at: String }
/// The four-field ranking/corpus revision summary a record carries.
pub struct RetrievalRevision { /* binary_version, schema_version, knobs_hash, corpus_digest */ }

// src/domains/learning/evaluation/dataset_rows.rs

/// Everything the manifest counts, accumulated as rows are built.
pub struct RowStats { /* the `counts` block */ }
/// Which provenance words reach the dataset.
pub enum ProvenanceFilter { Manual, Implicit, All }
/// The inputs one build needs, bundled rather than passed positionally.
pub struct RowInput<'a> { /* observations, candidates, judgments, filter knobs */ }
/// Build every exportable row, applying steps 1-7 of the pipeline.
pub fn build(input: RowInput<'_>) -> Result<(Vec<PendingRow>, RowStats)>;

// src/domains/learning/evaluation/dataset_dedup.rs

/// Collapse duplicate observations, keeping one per
/// `(query, knobs_hash, corpus_digest, filters_json)`.
pub fn collapse_duplicates(/* ... */) -> (Vec<String>, u64);
/// Resolve contradictory judgments within one provenance class.
pub fn resolve_contradictions(/* ... */) -> u64;

// src/domains/learning/evaluation/dataset_split.rs

/// The split configuration one export ran under.
pub struct SplitConfig { pub seed: String, pub ratios: [f64; 3],
    pub holdout_repo: Option<String>, pub holdout_since: Option<String> }
/// One group's assignment, as the manifest reports it.
pub struct GroupAssignment { /* group_id, split, forced, queries, contents, rows */ }
/// The bag-of-words key for one query.
pub fn query_group(query: &str) -> String;
/// The version-free, position-free content key for one identity.
pub fn content_group(identity: &CandidateIdentity) -> String;
/// Assign every row to a connected component and every component to a split.
pub fn plan(rows: &[PendingRow], cfg: &SplitConfig) -> Result<SplitPlan>;

// src/domains/learning/dataset_export.rs

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request { /* one field per flag */ }
/// What one export wrote, for the CLI to render.
pub struct ExportReport { pub out: PathBuf, pub manifest: DatasetManifest }
/// Read the window, build the dataset, write the directory, return the report.
pub fn run(ctx: &mut Ctx<'_>, req: &Request) -> Result<ExportReport>;
```

### The deterministic pipeline

Applied in exactly this order. Every step is total: it either admits a row,
or drops it and increments one named counter.

1. **Window.** Load observations with `at` in `[since, until)`, ascending by
   `(at, observation_id)`.
2. **Contract version.** An observation whose `observation_version` is not
   `OBSERVATION_VERSION` is refused: dropped, counted as
   `observations_refused_version`, and warned through `tracing`. Its candidates
   and judgments are never read.
3. **Unresolved candidates.** A candidate with `unresolved = 1` is dropped and
   counted as `candidates_unresolved`. A judgment naming it is additionally
   counted as `judgments_on_unresolved_candidates`. An unresolved candidate is
   never emitted, labeled or unlabeled, whatever its verdict says.
4. **Domain filter.** A candidate outside `--domain` is dropped and counted as
   `candidates_filtered_by_domain`.
5. **Duplicate observations.** Two observations sharing
   `(query, knobs_hash, corpus_digest, filters_json)` saw the same corpus under
   the same configuration for the same question. Keep one: the one with the most
   judgments still exportable after steps 3 and 4 and inside the selected
   provenance set, then the greatest `at`, then the greatest `observation_id`.
   Losers are dropped whole and counted as `duplicate_observations_dropped`.
   The collapse runs after the two drops, and not before, so a verdict that can
   never be exported — one on a redacted candidate, or on a domain this run
   excludes — cannot decide which observation survives.
6. **Judgment classification.** For each judgment on a surviving observation:
   - its `candidate_ref` equals a surviving candidate's → **matched**;
   - it does not, but some surviving candidate has the same identity keys under
     a different content version → **stale**, dropped, counted
     `judgments_stale`;
   - neither → **recall miss**, dropped, counted `judgments_recall_miss`.
   A stale or missing judgment is never inserted as a positive: it names a
   candidate this observation did not return at that version.
7. **Contradictions.** Group matched judgments by
   `(query_group, candidate_ref, provenance)`. When one key carries more than
   one distinct `relevance`, the judgment with the greatest `at` wins, ties
   broken by the greatest `observation_id`. Every losing row is dropped whole —
   not downgraded to unjudged, which would be a second kind of guess — and
   counted as `judgments_contradictory_dropped`. Manual and implicit never
   contend: they are different evidence classes and live in different files.
8. **Grouping and splitting.** Build one node per distinct `query_group` and
   one per distinct `content_group`; union the two endpoints of every surviving
   row. Each component's `group_id` is `g-` plus the first sixteen hex
   characters of `sha256` over its member keys sorted and newline-joined. A
   component containing a code candidate in `--holdout-repo`, or an observation
   at or after `--holdout-since`, is forced to holdout. Every other component
   takes `u = be_u64(sha256(seed || "\u{0}" || group_id)[0..8]) / 2^64` and the
   cumulative ratios.
9. **Bucketing and negative selection.** Rows are bucketed by
   `(split, label class)`. Only then, and only inside one bucket, does
   `--max-negatives-per-query` drop reviewed relevance-`0` rows beyond the cap
   for one `query_group`, keeping the lowest `pool_position` first; drops are
   counted as `negatives_capped`. No other selection, sampling or mining exists,
   and no step after 8 can read a row from another split.

## Failure modes and edge cases

| Case | Behavior |
| --- | --- |
| No observations in the window | Success. `train.jsonl` and `validation.jsonl` are written empty, the manifest reports zeros, and the TTY view says that nothing was captured and names `COMEMORY_OBSERVATIONS_ENABLED` |
| Observations exist, none judged | Success with zero labeled rows. Without `--include-unjudged` every file is empty and `candidates_unjudged` carries the whole pool, which is the "report missing labels" path |
| An observation at an unknown `observation_version` | Refused: dropped, counted, warned. The export still succeeds over the rest, because a bulk tool that refuses everything over one unreadable row is unusable, and the manifest count plus the TTY line make the omission visible |
| A candidate whose `candidate_ref` this build cannot parse | The whole export fails with `Error::Other` naming the observation and the pool position. A row the contract's own codec rejects is a corrupt record, not a filterable one, and silently dropping it would under-report the pool |
| An observation whose `filters_json` or `retrieval_json` this build cannot deserialize | The whole export fails with `Error::Json` naming the observation and the column. Both are contract fields a record carries verbatim; unlike the locator, neither is display-only, so neither may be dropped to keep going |
| An owned output name that is absent from `--out` | The removal is a no-op. Only `ErrorKind::NotFound` is ignored; any other removal failure — a directory under an owned name, a permission error — propagates as `Error::Io` naming the path, because a name the export could not clear is a name it cannot honestly claim to own |
| A judgment on an unresolved candidate | Never exported. Counted twice: once as `candidates_unresolved`, once as `judgments_on_unresolved_candidates` |
| A judgment naming a candidate the observation never returned | Dropped, counted `judgments_recall_miss`. Never appended to the pool |
| A judgment pinned to a content version the observation did not see | Dropped, counted `judgments_stale`. Never treated as a match |
| A truncated observation | Exported. Every one of its rows carries `"pool_truncated": true` and the manifest counts `observations_truncated`, so a consumer knows not to compute pool recall from it |
| Two observations of one query under one configuration | Collapsed to the most-judged one by step 3; the rest counted |
| The same candidate judged `3` and later `0` for the same query group | The later verdict wins; the earlier row is dropped and counted |
| Ratios that do not sum to `1.0`, are negative, or are not finite | `Error::Usage` naming the flag, before the database opens |
| An unknown `--provenance` word | `Error::Config` naming the accepted words, before the database opens |
| `--out` exists and holds an unrelated file | Untouched. Only the seven owned names are removed |
| `--out` exists and holds a stale `holdout.jsonl` from an earlier run | Removed before writing, so a withholding run cannot leave readable qualification rows behind |
| `--out` is an existing regular file | `Error::Io` from `create_dir_all`, naming the path |
| A partial write (disk full mid-file) | `Error::Io` propagates. The manifest is written last, so an export whose manifest is present is an export whose files are complete |
| Every group lands in one component | Success, and the manifest shows `groups: 1` with the whole row count in `largest_group_rows`. The operator sees that the corpus cannot be split rather than reading an implausible score later |
| `--holdout-repo` names a repo with no candidates | No group is forced. Recorded verbatim in the manifest so the mistake is visible |
| A holdout group exists but `--include-holdout` is absent | No holdout file is written; the manifest's `withheld` entry carries its row count and SHA-256 |

## Acceptance criteria

- **AC-1:** Given a real database holding captured observations across memory,
  code and document candidates with `comemory judge` verdicts on one of each,
  `comemory export-dataset --out <dir>` writes `train.jsonl`,
  `validation.jsonl` and `manifest.json`, and every emitted line parses as a
  `DatasetRecord` carrying `record_version`, `observation_version`, the query
  text, the candidate text, `candidate_ref`, the parsed `identity`,
  `content_version`, `filters`, `label.provenance`, `observation_id` and
  `retrieval_revision.{knobs_hash, corpus_digest, binary_version,
  schema_version}`.
- **AC-2:** With the default `--provenance manual`, an implicit judgment
  recorded through `judge::Request { source: Some("implicit"), .. }` appears in
  no emitted file. With `--provenance all` the same judgment appears only in
  `<split>.implicit.jsonl`, carrying `"provenance": "implicit"`, and never in
  `<split>.jsonl`.
- **AC-3:** A candidate judged relevance `0` is emitted with
  `"label": {"relevance": 0, ...}`. A retrieved candidate with no verdict is
  emitted only under `--include-unjudged`, and then with `"label": null`; it is
  never emitted with a relevance of `0`. The manifest's
  `counts.candidates_unjudged` reports the unjudged population in both runs.
- **AC-4:** A memory purged by `comemory gc` leaves its captured candidate
  `unresolved`; that candidate appears in no emitted file even though a
  judgment was recorded against it before the purge, and the manifest reports
  it under `counts.candidates_unresolved` and
  `counts.judgments_on_unresolved_candidates`.
- **AC-5:** Two observations of the same query under the same `knobs_hash` and
  `corpus_digest` collapse to one; the kept observation is the one carrying
  more judgments, and `counts.duplicate_observations_dropped` is `1`. Given the
  same candidate judged `3` against an earlier observation and `0` against a
  later one under one query group, only the `0` is emitted and
  `counts.judgments_contradictory_dropped` is `1`.
- **AC-6:** No `group_id` appears in more than one split file; two chunks of one
  document, two symbols of one file and two content versions of one memory
  always share a `group_id`; and two queries differing only in case,
  punctuation or word order share a `query_group`.
- **AC-7:** Without `--include-holdout`, `<dir>/holdout.jsonl` does not exist,
  no `candidate_ref`, no `text_sha256` and no `query` belonging to a holdout
  group appears in `train.jsonl` or `validation.jsonl`, and the manifest's
  `withheld` entry carries that file's row count and SHA-256. A prior run's
  `holdout.jsonl` left in `--out` is removed before the withholding run writes
  anything.
- **AC-8:** With `--max-negatives-per-query 1`, every query group in every
  emitted file contributes at most one reviewed relevance-`0` record, and
  `train.jsonl` and `validation.jsonl` are byte-identical between a run that
  passed `--include-holdout` and one that did not — so the selection that ran
  could not have read, counted or been influenced by a holdout row.
- **AC-9:** Running the same command twice against an unchanged database
  produces byte-identical `train.jsonl`, `validation.jsonl` and `manifest.json`,
  and the same `dataset_id` and `snapshot_digest`. Capturing one further
  observation for a query and a set of candidates that share no group key with
  any existing row changes `snapshot_digest` and `dataset_id` but leaves every
  previously assigned group on the split it already had.
- **AC-10:** An observation whose `observation_version` is not
  `OBSERVATION_VERSION` contributes no record, is counted under
  `counts.observations_refused_version`, and does not fail the run; a judgment
  naming a candidate the observation never returned is counted under
  `counts.judgments_recall_miss` and contributes no record.
- **AC-11:** No emitted record, in any run, carries a `CandidateLocator` field.
  The title each captured candidate was displayed under — read back from
  `candidate_observations.locator_json` — appears in no output file, and
  `store::candidate_dataset`'s SQL names no `locator_json` column.
- **AC-12:** `--holdout-repo <label>` forces every group containing a code
  candidate from that repo into the holdout split regardless of its hash, and
  the manifest records `split.holdout_repo` and marks those assignments
  `"forced": "repo"`.

## Acceptance evidence

| AC | Real input | Expected observable result | Boundary / failure case | Runnable check |
| --- | --- | --- | --- | --- |
| AC-1 | Three memories saved through the binary, a real git repo indexed with `index-code`, a markdown tree indexed with `index`, one capturing `comemory find`, three `comemory judge` verdicts | Three files exist; every line parses and carries all eleven named fields | Zero observations → empty files, manifest zeros | `tests/cli__export_dataset.rs::export_writes_a_reviewed_dataset_over_every_domain` |
| AC-2 | The same corpus. `comemory judge` cannot mint an implicit verdict by design — it has no `--source` flag — so the test writes one `candidate_judgments` row with `provenance = "implicit"` through `store::candidate_judgments::upsert_all` against the live database the binary just captured into, naming a `candidate_ref` that run really observed | Default run: absent. `--provenance all`: present only in `train.implicit.jsonl`, never in `train.jsonl` | `--provenance implicit` with no implicit rows → empty implicit files | `tests/cli__export_dataset.rs::implicit_labels_stay_out_of_the_reviewed_files` |
| AC-3 | One candidate judged `0`, ninety-odd unjudged | `label.relevance == 0` present; unjudged absent by default, `label: null` with the flag | An observation with no verdicts at all | `tests/cli__export_dataset.rs::a_reviewed_zero_is_a_hard_negative_and_silence_is_not` |
| AC-4 | Capture, judge, `comemory delete` + `comemory gc` on that memory | The redacted candidate is in no file; both counters are `1` | The same candidate also judged | `tests/cli__export_dataset.rs::a_purged_memory_is_reported_not_exported` |
| AC-5 | Two `find` runs of one query with capture on, judgments on one; then contradictory verdicts across two observations of one query group | One observation survives; the later verdict survives; both counters are `1` | Equal `at` values → the greater `observation_id` wins | `src/domains/learning/evaluation/tests/dataset_dedup.rs` |
| AC-6 | A document indexed into several chunks, a source file with several symbols, one memory edited and rebuilt | One `group_id` per component; disjoint split membership; the three query spellings share a key | A corpus that collapses into one component | `src/domains/learning/evaluation/tests/dataset_split.rs` |
| AC-7 | The full mixed corpus, run once with and once without `--include-holdout` | The withholding run writes no holdout file; the ref, digest and query sets are disjoint; `withheld[0].sha256` equals the included run's file digest | A stale `holdout.jsonl` planted in `--out` before the withholding run | `tests/cli__export_dataset.rs::the_holdout_split_is_unreadable_from_a_training_export` |
| AC-8 | Reviewed `0` verdicts on several candidates of one query | At most one negative per query group per file; `train.jsonl` and `validation.jsonl` byte-identical across a `--include-holdout` run and a withholding run | Cap `0` → no capping | `tests/cli__export_dataset.rs::negative_selection_runs_inside_a_split` |
| AC-9 | Any populated database | Two runs, byte-identical files and equal `dataset_id`; a third run after capturing a query that shares no group key with any existing row changes the id but no existing assignment | The added observation joining two components, which the manifest shows as a merged group | `tests/cli__export_dataset.rs::two_exports_of_one_snapshot_are_byte_identical` |
| AC-10 | An observation row rewritten to `observation_version = 999`; a judgment row inserted for an absent `candidate_ref` | Both counted, neither exported, exit `0` | Every observation refused → empty dataset, non-zero counter | `src/domains/learning/tests/dataset_export.rs` |
| AC-11 | The mixed corpus, whose memory titles are distinctive strings. The expected titles are read back out of `candidate_observations.locator_json` rather than hard-coded, so the check cannot pass by asserting against a string the corpus never used | No output file contains any of those titles; the store module's SQL names no `locator_json` | A locator whose title is empty, which is skipped rather than matching everything | `tests/cli__export_dataset.rs::no_display_field_reaches_a_training_record`, `src/store/tests/candidate_dataset.rs` |
| AC-12 | Two repos indexed, verdicts in both | Every group touching the named repo is `holdout` and `forced: "repo"` | A repo label with no candidates | `src/domains/learning/evaluation/tests/dataset_split.rs` |

Each colocated suite runs against a real SQLite database opened through
`store::connection::open` and populated through `store::candidate_observations::insert`
and `store::candidate_judgments::upsert_all`. Each `tests/cli__export_dataset.rs`
case drives the real binary with `assert_cmd` over the mixed corpus
`tests/common/observation_corpus.rs` already builds for `tests/cli__judge.rs` —
three memories saved through the production save path, one real git repository
indexed with `index-code`, one real markdown tree indexed with `index` — so the
observations the export reads were produced by a real `comemory find`. No mock
rows are constructed and no second corpus fixture is added.

## Documentation impact

- `docs/scenarios/export-dataset.md` — new, one scenario per flag with the test
  that covers it; required by `tests/cli_scenario_catalog.rs`.
- `docs/cli-reference.md` — regenerated by `bash scripts/regen-cli-docs.sh`.
- `docs/designs/2026-09-17-domain-first-migration-inventory.md` — one row per
  new production file, with the `bridge` cell naming each colocated suite.
- `README.md` — the new command in the command table and the learning-loop
  section.
- `AGENTS.md` — the module map rows for `domains/learning`, `store/` and `cli/`.
- `src/domains/learning/README.md`, `src/domains/learning/evaluation/README.md`,
  `src/store/README.md`, `src/cli/README.md` — the per-file indexes.
- `docs/guides/ranking-and-eval.md` — how a reviewed dataset is exported and
  what the splits mean, beside the existing benchmark and judge material.
- `tests/common/observation_corpus.rs` — reused unchanged; no second corpus
  fixture is introduced.

No environment variable and no config key is added: every knob is a flag, so
`src/config/` is untouched.

## Open Questions

1. **Should the query group key use SimHash rather than a token bag?** Resolved
   for this slice: the token bag is used, because SimHash needs a radius and a
   radius makes group membership depend on the rest of the corpus, which makes
   split assignment unstable under growth. Owner: this design. Non-blocking —
   the key is computed in one function and the manifest records `split.policy`,
   so a future policy can be added beside it under a new policy name.
2. **Should a contradictory judgment be resolved by recency or by majority?**
   Resolved: recency, matching `candidate_judgments`'s own "a re-judgment
   replaces the row" rule extended across observations. Owner: this design.
   Non-blocking.
3. **Should #214 consume one file per split or one file with a `split`
   field?** Resolved: one file per split, with the `split` field also present on
   every record. The file boundary is what makes withholding provable; the field
   is what makes a concatenated file safe. Owner: shared with #214.
   Non-blocking.
