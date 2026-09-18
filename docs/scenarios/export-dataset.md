# `comemory export-dataset`

Write a versioned JSONL relevance dataset and its manifest from the candidate
observations [`find`](find.md) captured and the verdicts [`judge`](judge.md)
recorded.

The default is the conservative one: reviewed (`manual`) verdicts only, the
retrieved pool nobody judged left out, and the holdout split withheld. A
reviewed relevance of `0` is a hard negative; a candidate nobody judged is
reported as unjudged and never becomes a zero. An unresolved candidate — one a
purge redacted or one whose row vanished mid-capture — is reported and never
exported, whatever verdict was recorded against it.

Splits are assigned to connected components of the query/content graph, so two
chunks of one document, two symbols of one file, two content versions of one
memory and two spellings of one query never land on opposite sides. The holdout
split is the qualification set: without `--include-holdout` no `holdout.jsonl`
is written at all, and the manifest publishes only its row count and SHA-256.

Repeating an export from an unchanged database is byte-identical, and the
manifest carries `dataset_id` and `snapshot_digest` to prove it.

**Runnable tests:** `tests/cli__export_dataset.rs`, `tests/cli__export_dataset_2.rs`

**HTTP:** none — CLI only. The run writes a directory of files to an
operator-named filesystem path, which a server must never do on request.

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--out` | required | Output directory, created if absent. Every file this command owns is removed from it before writing; nothing else is touched |
| `--provenance` | `manual` | `manual` \| `implicit` \| `all`. Implicit labels are written to their own `<split>.implicit.jsonl` and never mixed into the reviewed files |
| `--include-unjudged` | off | Also emit a record per retrieved candidate with no verdict, carrying `label: null` |
| `--include-holdout` | off | Also write the holdout split's files |
| `--domain` | all three | Restrict records to `memory`, `code` or `document`. Repeatable |
| `--since` | unset | Only observations captured at or after this instant |
| `--until` | unset | Only observations captured before this instant |
| `--split` | `0.7,0.15,0.15` | Train, validation and holdout ratios, summing to `1.0` |
| `--split-seed` | `comemory-dataset-v1` | Salt for the group hash; changing it reassigns every group |
| `--holdout-repo` | unset | Reserve every group holding a code candidate from this repo for the holdout split |
| `--holdout-since` | unset | Reserve every group holding an observation captured at or after this instant |
| `--max-negatives-per-query` | `0` | Cap reviewed relevance-`0` records per query group per split. `0` is unlimited |

`--json` prints the manifest: the two digests, the filtering configuration, the
split plan with every group assignment, the per-domain, per-split and per-label
counts, the retrieval revisions, the files written and the files withheld.

## Scenarios

### export-dataset-01 A reviewed dataset over every domain

- **Flags:** `--out` `--split` `--json`
- **Setup:** three memories saved through the binary, a real git repo indexed
  with `index-code`, a markdown tree indexed with `index`, one capturing
  `comemory find`, and one `comemory judge` verdict per domain
- **Command:** `comemory --json export-dataset --out ./dataset --split 1,0,0`
- **Expect:** `train.jsonl`, `validation.jsonl` and `manifest.json` exist and
  `holdout.jsonl` does not; one record per verdict; every record carries
  `record_version`, `observation_version`, the query, the passage and its
  digest, the parsed `identity`, the `content_version`, the effective
  `filters`, `label.provenance` and the four-field `retrieval_revision`.
- **Covered by:** `tests/cli__export_dataset.rs::export_writes_a_reviewed_dataset_over_every_domain`

### export-dataset-02 Implicit labels stay separate

- **Flags:** `--provenance`
- **Setup:** the same corpus, plus one `candidate_judgments` row written with
  `provenance = 'implicit'` against a candidate that run really observed
- **Command:** `comemory --json export-dataset --out ./both --provenance all`
- **Expect:** the default run writes no implicit file and no implicit record;
  `--provenance all` writes the implicit record only to
  `train.implicit.jsonl`, carrying `"provenance": "implicit"`, and never into
  `train.jsonl`.
- **Covered by:** `tests/cli__export_dataset.rs::implicit_labels_stay_out_of_the_reviewed_files`

### export-dataset-03 A reviewed zero is a hard negative; silence is not

- **Flags:** `--include-unjudged`
- **Setup:** a captured pool with one candidate graded `0` and the rest unjudged
- **Command:** `comemory --json export-dataset --out ./with-pool --include-unjudged`
- **Expect:** the reviewed-irrelevant candidate is a record with
  `label.relevance == 0`; without the flag the unjudged pool produces no
  records and is reported under `counts.candidates_unjudged`; with it every
  extra record carries `label: null` and never a relevance of `0`.
- **Covered by:** `tests/cli__export_dataset.rs::a_reviewed_zero_is_a_hard_negative_and_silence_is_not`

### export-dataset-04 A purged memory is reported, not exported

- **Flags:** `--include-unjudged`
- **Setup:** capture, judge, then `comemory delete` plus `comemory gc` on the
  judged memory, which redacts its captured passage
- **Command:** `comemory --json export-dataset --out ./dataset --include-unjudged`
- **Expect:** the redacted candidate appears in no file; the manifest reports
  it under `counts.candidates_unresolved` and its verdict under
  `counts.judgments_on_unresolved_candidates`; the rest of the pool is intact.
- **Covered by:** `tests/cli__export_dataset.rs::a_purged_memory_is_reported_not_exported`

### export-dataset-05 The qualification split is unreadable from a training export

- **Flags:** `--holdout-repo` `--include-holdout`
- **Setup:** the mixed corpus with a second, lexically isolated query so more
  than one component exists, and a stale `holdout.jsonl` planted in `--out`
- **Command:** `comemory --json export-dataset --out ./training --holdout-repo demo`
- **Expect:** the planted file is removed and no `holdout.jsonl` is written;
  the manifest's `withheld` entry carries its row count and SHA-256; no
  `candidate_ref`, `text_sha256`, `query` or `group_id` of a holdout row
  appears in `train.jsonl` or `validation.jsonl`. Releasing it with
  `--include-holdout` reproduces exactly that digest.
- **Covered by:** `tests/cli__export_dataset_2.rs::the_holdout_split_is_unreadable_from_a_training_export`

### export-dataset-06 Negative selection runs inside a split

- **Flags:** `--max-negatives-per-query`
- **Setup:** reviewed `0` verdicts on several candidates of one query
- **Command:** `comemory --json export-dataset --out ./capped --holdout-repo demo --max-negatives-per-query 1`
- **Expect:** no query group contributes two reviewed negatives, and
  `train.jsonl` and `validation.jsonl` are byte-identical whether or not
  `--include-holdout` was passed — so the selection could not have read a
  holdout row.
- **Covered by:** `tests/cli__export_dataset_2.rs::negative_selection_runs_inside_a_split`

### export-dataset-07 Two exports of one snapshot are byte-identical

- **Flags:** `--out`
- **Setup:** any populated database
- **Command:** `comemory --json export-dataset --out ./dataset --include-unjudged`
- **Expect:** the second run's `train.jsonl`, `validation.jsonl` and
  `manifest.json` are byte-identical and carry the same `dataset_id` and
  `snapshot_digest`; one further capture changes both.
- **Covered by:** `tests/cli__export_dataset_2.rs::two_exports_of_one_snapshot_are_byte_identical`

### export-dataset-08 No display field becomes a training key

- **Flags:** `--out`
- **Setup:** a captured pool whose candidates carry real locators
- **Command:** `comemory export-dataset --out ./dataset --include-unjudged`
- **Expect:** no stored `locator_json` body appears in any output file, and no
  record carries a `title`, `heading_path`, `symbol_id`, `line_range` or
  `locator` key at any depth. `store::candidate_dataset` never selects the
  column.
- **Covered by:** `tests/cli__export_dataset_2.rs::no_display_field_reaches_a_training_record`,
  `src/store/tests/candidate_dataset.rs::the_projection_carries_the_passage_and_never_the_locator`

### export-dataset-09 The manifest reports the configuration and the plan

- **Flags:** `--domain` `--since` `--until` `--split-seed` `--holdout-since` `--json`
- **Setup:** the mixed corpus
- **Command:** `comemory --json export-dataset --out ./dataset --domain memory --domain code --split 0.5,0.25,0.25 --split-seed pinned`
- **Expect:** `manifest_version`, `record_version`, the filtering
  configuration verbatim, `split.policy` `grouped-hash` with its seed, ratios
  and per-group assignments, the per-label counts, and the schema version the
  observations were captured under. A `--split` that does not sum to `1.0` is
  refused naming the rule. `--since` / `--until` narrow the observation window
  and `--holdout-since` reserves a time slice, both on
  `candidate_query_observations.at`.
- **Covered by:** `tests/cli__export_dataset_2.rs::the_manifest_reports_the_filtering_configuration_and_the_split_plan`,
  `src/store/tests/candidate_dataset.rs::the_window_is_half_open_on_both_sides_and_each_bound_is_optional`,
  `src/domains/learning/evaluation/tests/dataset_split.rs::a_reserved_time_slice_forces_every_component_inside_it`

### export-dataset-10 Duplicates, contradictions and an unknown contract version

- **Flags:** `--out`
- **Setup:** two observations of one query under one ranking configuration, a
  candidate judged `3` and later `0` under one query group, and an observation
  row written at `observation_version = 999`
- **Command:** `comemory --json export-dataset --out ./dataset`
- **Expect:** the most-judged duplicate survives and
  `counts.duplicate_observations_dropped` is `1`; the later verdict wins and
  `counts.judgments_contradictory_dropped` is `1`; the unknown contract version
  is refused, counted under `counts.observations_refused_version`, and does not
  fail the run.
- **Covered by:** `src/domains/learning/tests/dataset_export.rs::duplicate_observations_collapse_and_a_revised_verdict_wins`,
  `src/domains/learning/evaluation/tests/dataset_rows.rs::an_unknown_contract_version_is_refused_and_counted_without_failing_the_run`
