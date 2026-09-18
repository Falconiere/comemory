# `comemory judge`

Record reviewed relevance verdicts against a candidate observation that
`comemory find` captured, or — with no verdict — report that observation.

A verdict addresses a candidate by its domain-qualified reference, the same
opaque token the reranker process protocol passes around, so a target the pool
never returned cannot be recorded at all. Matching goes through the typed
identity, never a title or a path shown on screen, and a reference pinned to a
content version this observation did not see is refused as **stale** rather
than silently accepted.

Recording is all-or-nothing: one unresolvable reference refuses the whole call
and writes nothing, including the references that did resolve.

Capture is opt-in. Set `observations.enabled` (or
`COMEMORY_OBSERVATIONS_ENABLED=1`) and `comemory find` records its candidate
pool and prints the observation id. See [find.md](find.md).

Distinct from [`feedback`](feedback.md), which keeps per-memory and per-symbol
counters keyed by a `retrieval_log` query id. That command, its tables and its
provenance vocabulary are unchanged; a judgment is a second, parallel record
that also covers documents, which `feedback` has never had a path for. Like
`feedback`, `judge` has no `--source` flag: a typed verdict is a human one and
is always stored as `manual` (#130).

**Runnable tests:** `tests/cli__judge.rs`, `tests/cli__judge_2.rs`

**HTTP:** none — CLI only. A verdict against a locally captured observation is
a local review action; the two HTTP feedback routes keep their own contract.

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

| Positional | Required | Effect |
| --- | --- | --- |
| `<OBSERVATION_ID>` | yes | The `o-<yyyymmdd>-<8hex>` id `comemory find` printed when candidate capture was armed |

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--ref` | unset, repeatable | One verdict, as `<candidate_ref>=<relevance>` with relevance in `0..=3`. With none, the observation is reported and nothing is written |

`--json` prints `{observation_id, recorded, provenance}` when recording, and
the full observation — query, contract version, both digests, and every
candidate with its pool and page positions — when reporting.

## Scenarios

### judge-01 A verdict in every domain, under the stated provenance

- **Flags:** `--ref` `--json`
- **Setup:** three memories saved through the binary, a real git repo indexed
  with `index-code`, a markdown tree indexed with `index`, then one capturing
  `comemory find`
- **Command:** `comemory --json judge o-… --ref 'memory:…=3' --ref 'code:…=2' --ref 'document:…=0'`
- **Expect:** three `candidate_judgments` rows, one per domain, all provenance
  `manual`; `recorded: 3`. The document verdict has no other path in comemory.
  `feedback`, `code_feedback` and `feedback_events` are untouched, and a later
  `comemory feedback` against the same run's query id still works.
- **Covered by:** `tests/cli__judge_2.rs::judgments_record_across_every_domain_under_the_stated_provenance`,
  `tests/cli__judge_2.rs::the_legacy_feedback_path_is_unchanged_by_a_judgment`

### judge-02 A target retrieval never returned

- **Flags:** `--ref`
- **Setup:** one captured observation
- **Command:** `comemory judge o-… --ref '<matching>=3' --ref '<absent>=3'`
- **Expect:** exit `78`, the message naming the absent reference as a
  candidate-pool recall miss, and **nothing** written — including the
  reference that did match.
- **Covered by:** `tests/cli__judge_2.rs::a_judgment_retrieval_never_returned_is_refused_and_nothing_is_written`

### judge-03 A changed memory is stale; a deleted one is a miss

- **Flags:** `--ref`
- **Setup:** capture, judge, then edit the memory's markdown body and
  `comemory rebuild`; later, `comemory delete` it
- **Command:** `comemory judge o-<later> --ref '<original>=3'`
- **Expect:** exit `78` with `is stale` while the identity still exists under a
  new content version, and `candidate-pool recall miss` once it is deleted. The
  verdict recorded against the original observation still stands, because that
  observation still holds the passage it showed.
- **Covered by:** `tests/cli__judge_2.rs::a_changed_memory_makes_a_pinned_judgment_stale_and_a_deleted_one_a_recall_miss`

### judge-04 Re-indexing that reuses code rowids

- **Flags:** `--ref`
- **Setup:** capture and judge a code candidate, then edit and commit the
  indexed file and re-run `index-code`, which purges and reinserts that file's
  `code_symbols` rows
- **Command:** `comemory judge o-<later> --ref '<pre-edit code ref>=3'`
- **Expect:** no `candidate_observations` or `candidate_judgments` value
  contains a `code_symbols` rowid; the stored judgment still names the same
  `(repo, path, symbol)`; the pre-edit `blob_oid` is refused as stale against a
  fresh observation rather than matching whatever symbol inherited the number.
- **Covered by:** `tests/cli__judge_2.rs::reindexing_that_reuses_code_rowids_never_moves_a_judgment`

### judge-05 A replaced document chunk

- **Flags:** `--ref`
- **Setup:** capture and judge a document candidate, rewrite the markdown file,
  re-run `comemory index`
- **Command:** `comemory judge o-<later> --ref '<pre-edit document ref>=2'`
- **Expect:** exit `78` with `is stale` (the parent's `revision_hash` moved);
  the first observation still holds the passage it actually showed, byte for
  byte.
- **Covered by:** `tests/cli__judge_2.rs::a_replaced_document_chunk_makes_the_old_ref_stale`

### judge-06 Delayed feedback

- **Flags:** `--ref` `--json`
- **Setup:** capture, then change the corpus (a new memory, an edited and
  re-indexed source file), and only then record the verdict
- **Command:** `comemory --json judge o-<original> --ref '<memory>=3' --ref '<code>=1'`
- **Expect:** both verdicts resolve against the corpus that observation saw and
  store the content version it saw; the same reference against a fresh
  observation is stale. A late verdict is never re-attributed to today's
  content.
- **Covered by:** `tests/cli__judge_2.rs::a_delayed_verdict_resolves_against_the_corpus_it_observed`

### judge-07 Reporting an observation

- **Flags:** `--json`
- **Setup:** one captured observation with one verdict already recorded
- **Command:** `comemory --json judge o-…`
- **Expect:** the query, `observation_version`, both digests, `truncated`, and
  every candidate with its pool position, page position, reference, title and
  any recorded relevance; an unjudged candidate reports `relevance: null`.
  Nothing is written. The TTY view names the observation and every reference.
- **Covered by:** `tests/cli__judge_2.rs::judge_without_a_verdict_reports_the_observation`

### judge-08 Refusals: unknown id, unknown contract version, redacted candidate

- **Flags:** `--ref`
- **Setup:** a captured observation, plus a purged memory and an observation
  whose `observation_version` was set to `999`
- **Command:** `comemory judge o-20260918-deadbeef`
- **Expect:** exit `69` (`EX_UNAVAILABLE`) naming an observation that does not
  exist; exit `78` (`EX_CONFIG`) for a malformed id, for an unknown
  `observation_version` (naming both versions), and for a candidate a purge
  redacted, which has no content snapshot to judge.
- **Covered by:** `tests/cli__judge_2.rs::an_unknown_or_malformed_observation_id_is_refused_with_its_own_code`,
  `tests/cli__judge_2.rs::an_unknown_observation_version_is_refused`,
  `tests/cli__judge.rs::purging_a_memory_redacts_its_captured_text_and_keeps_the_pool_shape`
