# Duplication debt baseline (`scripts/dup-check.sh`)

Status: documented baseline, tracked by a count ratchet against a **pinned**
`similarity-rs` · Owner: whoever burns a pair down next

**286 near-duplicate function/method pairs at threshold 0.85**, measured with
**`similarity-rs 0.5.0`** over the 530 production `.rs` files under `src/`. That
number and the tool that produced it are recorded together, here and in
`dup-baseline.txt`, because either one alone is meaningless.

**Why this number rose from 285 (#253).** Document replication added the pulled
cache, its search half and the acceptance path. The raw increase was **eleven**
pairs and **ten were burned down**, so exactly one is inherited:

- `retrieval/doc_route.rs` (1 inherited). `local_hit` and `shared_hit` resolve
  one search hit into the same `Resolved`, and report against each other because
  both are "look the rows up, or give up". They cannot be one implementation:
  one reads `documents`/`document_chunks`, the other `remote_document`/
  `remote_document_chunk`, and keeping those two table sets apart is the whole
  premise of the feature — an import may not touch a local row. A single
  resolver would have to branch on the side internally, which is the same two
  bodies with a worse name.

The ten burned, for the next reader: `store/remote_document.rs` gave up six —
the header insert moved into the one function that writes it, a passage and its
searchable row are written together, `purge` became a loop over the four tables
that share one key, and the two child readers became one `all::<T>` over a
`PulledRow` trait. `sync/replica/validate.rs` gave up three, including the pair
this file previously recorded as irreducible (see below): adding a third kind
made the decode-and-refuse skeleton worth extracting, so each kind now states
only its invariant through an `Identity` trait. `store/remote_document_view.rs`
gave up one by reading a pulled passage through the typed reader that already
existed rather than a second query of its own.

**Why this number rose from 275 (#252).** Code-generation replication added
four declared tables and the store modules that read and write them, which is
the single largest grandfathered class here — per-table row mappers. The raw
increase was **fifteen** pairs; **five were burned down before recording the
rest**, and only the ten below are inherited:

- `domains/code/generation.rs` (3 burned). `symbols_of` / `imports_of` /
  `co_changes_of` had been split out purely to keep `project` under the
  50-line ceiling, which made three same-shaped map-to-wire helpers. The symbol
  walk moved back into `project` where the manifest it belongs to is built, and
  the two edge sources became one `edges_of`, since "every edge the local index
  can state" is one idea.
- `store/rebuild_copy_code.rs` (2 burned). Every copy here was
  `if old_table_exists(…) { execute_batch(…) }`; the guard is now stated once
  in `copy_if_present`, and `copy_code_generations` no longer reports against
  any of its neighbours.
- `sync/replica/validate.rs` (1 burned, 1 inherited — **the inherited pair was
  burned in #253**). `payload_shape` and `code_payload_shape` ran the same three
  checks — payload-vs-tombstone, digest-covers-bytes, decode — before diverging,
  so they collapsed into one `shape` taking the kind's identity rule. The two
  identity rules were then recorded here as irreducible. That held while there
  were two of them; a third kind made the decode-and-refuse skeleton worth
  extracting after all, so they are now one generic `identity::<T>` over an
  `Identity` trait and each kind states only its own contract.
- `store/remote_code.rs` (1 burned of 6, 5 inherited). `files` / `symbols` /
  `edges` became one `projection` reader — a generation is written and read as
  a unit, so three readers that could be called out of step with each other
  were wrong independently of this count. What remains is irreducible: three
  `read` row-mappers, one per projection row type, and `projection` reporting
  against `replace_generation` and `purge_generation`, which are the select,
  insert and delete halves of the same three tables.
- `store/code_generation.rs` (3 inherited). `active`, `active_local` and `all`
  are 3-to-10-line accessors over one table returning `Option` and `Vec`.
  `active_local` could be inlined as `active(…).filter(…)` at its one call
  site, which would clear two of the three — it is kept because "the active
  generation this machine built" is the invariant that prevents a replication
  loop, and spreading it into the caller to satisfy a similarity score would
  trade a named contract for a number.

None of the ten is a case where one implementation could serve both callers.
A raise is not the ratchet's normal direction: it is recorded here rather than
left silent so the next burn-down knows exactly which rows it inherited.

**Why this number rose from 266.** The activity feed added per-table query
projections and short route adapters — the two largest grandfathered classes
below — and the 300-line ceiling split `config/validate.rs`'s per-knob bound
checks into `config/validate_knobs.rs`, which re-reported two pairs under the
new path and surfaced three more among functions that had not moved at all.
Every one of the nine is enumerated below with its rationale; none is a case
where one implementation could serve both callers. A raise is not the ratchet's
normal direction: it is recorded here rather than left silent so the next
burn-down knows exactly which rows it inherited.

## History: why the previous baseline had no authority

`scripts/dup-check.sh` was silently non-functional before Phase 6 of the
toolu-conventions migration: it invoked `similarity-rs` with CLI flags the
installed version doesn't accept and grepped for output shapes the tool never
produces, so it always fell through to a green no-op. Phase 6 fixed the
invocation and recorded **113** pairs. That 113 then stood until issue #200,
and four independent measurements showed it could not be trusted:

1. **It did not reproduce on its own tree.** Re-running `similarity-rs 0.5.0`
   against the tree at `16286113` — the commit that wrote `113` into
   `dup-baseline.txt` — reports **140** pairs over that tree's 226 production
   `.rs` files (its 45 `scripts/*.sh` were passed and, as always, ignored). The
   baseline had been taken with a different `similarity-rs` build, and the
   script pinned no version, so every comparison it fed was apples-to-oranges.
2. **Nothing ran the gate.** `dup-check.sh` appeared in neither
   `scripts/check-all.sh`'s `GATES` array nor any file under
   `.github/workflows/`. Its only caller was the `justfile`'s opt-in `qa`
   recipe. A ratchet nobody runs, over a tool nobody pins, is how the gap went
   unnoticed across sixty merged pull requests.
3. **It skipped silently when the tool was absent**, logging a dim info line
   and exiting 0 — a guardrail reporting success while checking nothing.
4. **This document contradicted itself.** It was headed "The 113 pairs" while
   its tables enumerated only 105 rows.

The #164 domain migration deliberately left all of this unfixed: re-baselining
to make the gate pass would have recorded a number with no more authority than
the one it replaced. Issue #200 pinned the tool, wired the gate into CI, and
re-baselined in one commit, which is what gives the number below its authority.
For the record, the migration itself added **no** duplication: 242 pairs on both
sides of every slice, with an empty normalized pair-set diff.

## What this baseline is measured with

- **Tool:** `similarity-rs 0.5.0`, pinned by the `SIMILARITY_RS_VERSION`
  constant in `scripts/dup-check.sh`. The gate **hard-fails** when a different
  version is on `PATH`, locally as well as in CI: a count from another build
  cannot be compared to `dup-baseline.txt` at all, so failing loudly beats
  passing wrongly.
- **Where CI gets it:** `.github/workflows/test.yml` *derives* the version from
  that same constant and `cargo install`s exactly it, so the workflow and the
  gate cannot drift. Bumping the constant is the only edit a version bump needs.
- **Threshold:** `0.85`, via `--threshold 0.85 --fail-on-duplicates`.
- **Scope:** every tracked `src/**/*.rs` whose path does not contain `/tests/`
  — 414 files. Colocated tests are excluded because CORE excludes tests from
  copy-paste detection: repeated setup there is not the duplication worth
  chasing.
- **Result when #200 fixed the gate:** 242 pairs over the then-current 411
  files. The count was deterministic: three consecutive runs of the pinned build returned 242 every
  time, as did runs with the file list shuffled and reversed, so the number does
  not depend on the order `git ls-files` returns.
- **Shell scripts are NOT covered — a limitation, not a clean result.**
  `similarity-rs` parses Rust and nothing else. Handed the 53 tracked
  `scripts/*.sh` paths alongside the Rust files it silently discarded them and
  reported `Checking 411 files`; run over those 53 alone it prints `No Rust
  files found in the specified paths.` and exits 0 even at threshold `0.5`.
  Earlier versions of this document and of the script claimed `src/ + scripts/`
  coverage, which was never true. The shell paths are therefore no longer passed
  at all: an inert path inflates the target count and would let the gate's
  anti-vacuous guard pass over files nobody analyzes. Duplication in `scripts/`
  is currently **unguarded**; catching it needs a shell-capable detector.
- **Current measurement:** 235 pairs over 414 files after the runtime ORM
  conversion and shared AST, configuration, HTTP-query and SSE helpers in #216.
  The unchanged detector first reported 278 on the conversion; shared store
  queries reduced it to 271, and broader cleanup reduced it to 235. The baseline
  tightens from 242 to 235; neither the threshold nor scan scope changed.
- **Every pair is intra-file.** No pair spans two files, so the rationale below
  is recorded once per file rather than once per pair.

Re-run the gate, or the raw scan underneath it, yourself:

```bash
bash scripts/dup-check.sh     # the gate: pinned-version check + ratchet
just qa                       # check-all + deny + dup + machete

# the raw scan the gate wraps
similarity-rs --threshold 0.85 --fail-on-duplicates \
  $(git ls-files 'src/*.rs' | grep -v '/tests/')
```

## How the gate treats this

`similarity-rs` has no native per-pair suppression flag (confirmed via
`similarity-rs --help` — there is `--exclude` for directories and
`--filter-function` / `--filter-function-body` for substring filters, nothing
addressing an individual reported pair). A per-pair allowlist would therefore
mean reimplementing pair identity matching by hand, which is worse than the
problem it solves. `scripts/dup-check.sh` runs a **count ratchet** instead:

- The baseline count (`275`) lives in `dup-baseline.txt`, a single tracked
  integer at the repo root — the same convention as `coverage-floor.txt` and
  `store-leak-baseline.txt`.
- Every run re-scans and compares the current count against that baseline.
- **Current count > baseline → the gate fails.** New duplication introduced by
  a future PR is caught, exactly as if the gate had always worked.
- **Current count <= baseline → the gate passes**, printing a nudge when the
  count came in lower. The pairs below are grandfathered, not hidden: they are
  enumerated in full, with the reason each is non-urgent debt.
- Burning a pair down lowers both `dup-baseline.txt` and the table below, in
  the same PR. The ratchet only ever tightens.

**A ceiling, not an exact match — deliberately one-sided.**
`scripts/store-chokepoint-check.sh` is the repo's *two-sided* ratchet, and its
header draws the contrast on purpose. That gate drives a counted debt to zero
under an active burn-down, so it must fail in both directions to force every PR
to record its own reduction. Duplication has no zero target — the largest
clusters here are documented deliberate per-language and per-table repetition —
and no burn-down campaign behind it, so failing a PR for refactoring unrelated
code *downward* would be friction with nothing to justify it.

**Where it runs, and why not in `check-all.sh`.** The gate is deliberately
absent from `scripts/check-all.sh`'s `GATES` array. It needs a separately
installed tool, and `check-all.sh` is what contributors run on every iteration
and what the release path runs; putting it there would mean either hard-failing
every contributor who lacks `similarity-rs` or skipping silently, and a
guardrail that reports success while checking nothing is the failure mode this
work exists to remove. It is grouped instead with the other two tool-dependent
gates — `deny-check.sh` and `machete-check.sh` — in the `justfile`'s `qa`
recipe, and runs authoritatively as its own step in `.github/workflows/test.yml`,
where the pinned version is guaranteed. Since #205 those other two are pinned
and wired the same way, each by its own constant, so all three are enforced
rather than advisory. When `similarity-rs` is missing, the
gate **fails in CI** and **skips locally with a loud warning** that names the
pinned install command and states plainly that it did not run.

## The 235 pairs by area

> **This table is stale and was already stale before #252.** It enumerates the
> 235 pairs measured at #216 and was not re-authored when the ratchet rose to
> 275, so its total no longer sums to `dup-baseline.txt`. The ratchet itself
> re-scans and does not read this table. Re-deriving it is its own task; the
> rows below remain accurate for the pairs they name.

Counts sum to the recorded baseline; if they ever stop matching, one of the two
is stale.

| Area | Pairs |
| --- | --- |
| `src/store/` | 99 |
| `src/config/` | 27 |
| `src/serve/routes/` | 26 |
| `src/domains/sync/` | 11 |
| `src/serve/` | 9 |
| `src/serve/routes/memories/` | 9 |
| `src/domains/integrations/setup/` | 8 |
| `src/domains/retrieval/` | 7 |
| `src/serve/jobs/` | 7 |
| `src/domains/graph/` | 6 |
| `src/domains/memories/` | 3 |
| `src/utilities/` | 3 |
| `src/` | 2 |
| `src/cli/` | 2 |
| `src/cli/output/` | 2 |
| `src/domains/code/ast/` | 2 |
| `src/domains/sync/exchange/` | 2 |
| `src/domains/sync/memory_store/` | 2 |
| `src/serve/routes/maint/` | 2 |
| `src/domains/code/` | 1 |
| `src/domains/documents/document/` | 1 |
| `src/domains/learning/` | 1 |
| `src/domains/maintenance/` | 1 |
| `src/domains/maintenance/doctor/` | 1 |
| `src/domains/retrieval/unified/` | 1 |
| **Total** | **235** |

## The 235 pairs, grouped by area

Grouped by directory. Similarity is the pinned detector's score at threshold
0.85; line ranges are function/method spans at scan time. The ratchet re-scans
fresh and does not depend on these line numbers.

### `src/store/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/store/fts_memory.rs:40-58` function `search_memory` | `src/store/fts_memory.rs:82-100` function `search_memory_subtokens` | 98.78% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/fts_memory.rs:40-58` function `search_memory` | `src/store/fts_memory.rs:63-73` function `search_memory_relaxed` | 98.09% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sync_log.rs:39-46` method `parse` | `src/store/sync_log.rs:69-75` method `parse` | 97.95% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/fts_memory.rs:63-73` function `search_memory_relaxed` | `src/store/fts_memory.rs:82-100` function `search_memory_subtokens` | 97.35% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sync_log.rs:149-151` function `entries_since` | `src/store/sync_log.rs:154-156` function `local_entries_since` | 96.76% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sync_log.rs:30-36` method `as_str` | `src/store/sync_log.rs:61-66` method `as_str` | 96.75% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/edges.rs:258-261` function `delete_outgoing` | `src/store/edges.rs:265-273` function `delete_touching` | 94.67% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sync_state.rs:52-62` function `set_pushed` | `src/store/sync_state.rs:65-75` function `set_pulled` | 94.57% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/vector.rs:58-60` function `insert_memory` | `src/store/vector.rs:69-71` function `replace_memory` | 94.02% | deliberate per-table repetition across the parallel vector tables |
| `src/store/vector.rs:164-166` function `insert_code` | `src/store/vector.rs:170-172` function `replace_code` | 94.00% | deliberate per-table repetition across the parallel vector tables |
| `src/store/sources.rs:123-133` function `row_from_sql` | `src/store/sources.rs:343-357` function `file_row_from_sql` | 93.26% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/documents.rs:83-100` function `get_document` | `src/store/documents.rs:208-216` function `get_document_path` | 92.85% | parallel document fetch helpers over twin lookups |
| `src/store/memory_meta.rs:95-112` function `attach_tags` | `src/store/memory_meta.rs:118-146` function `attach_references` | 92.72% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sources.rs:111-121` function `select_roots` | `src/store/sources.rs:327-341` function `select_files` | 92.64% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/orm.rs:54-60` function `query_one` | `src/store/orm.rs:63-69` function `query_optional` | 92.27% | shared preparation and binding with distinct collection, required-row and optional-row contracts |
| `src/store/eval_runs.rs:218-220` function `set_applied` | `src/store/eval_runs.rs:224-226` function `set_discarded` | 92.20% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/edges.rs:349-370` function `memory_direct_relation_edges` | `src/store/edges.rs:375-389` function `memory_co_citation_edges` | 92.17% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/orm.rs:41-51` function `query_all` | `src/store/orm.rs:54-60` function `query_one` | 92.01% | shared preparation and binding with distinct collection, required-row and optional-row contracts |
| `src/store/repo_marker.rs:75-77` function `last_mined_commit` | `src/store/repo_marker.rs:94-96` function `last_head` | 92.01% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/orm.rs:41-51` function `query_all` | `src/store/orm.rs:63-69` function `query_optional` | 91.73% | shared preparation and binding with distinct collection, required-row and optional-row contracts |
| `src/store/vector.rs:48-50` function `dim_memory` | `src/store/vector.rs:53-55` function `dim_code` | 91.14% | deliberate per-table repetition across the parallel vector tables |
| `src/store/code_ref.rs:46-59` function `materialize` | `src/store/code_ref.rs:85-104` function `upsert` | 90.84% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/memory_row.rs:208-227` function `mined_edges` | `src/store/memory_row.rs:233-250` function `restore_mined_edges` | 90.75% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/code_feedback.rs:45-57` function `own_identity` | `src/store/code_feedback.rs:61-70` function `parent_identity` | 90.59% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/sources.rs:64-77` function `upsert` | `src/store/sources.rs:206-224` function `upsert_file` | 90.50% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/documents.rs:83-100` function `get_document` | `src/store/documents.rs:176-201` function `get_chunk` | 90.40% | parallel document fetch helpers over twin lookups |
| `src/store/memory_meta.rs:199-225` function `fetch_extra` | `src/store/memory_meta.rs:252-280` function `rank_signals` | 90.07% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sources.rs:97-103` function `get` | `src/store/sources.rs:265-271` function `get_file` | 90.05% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/rebuild_copy_documents.rs:53-59` function `copy_document_chunks` | `src/store/rebuild_copy_documents.rs:63-69` function `copy_document_fts` | 89.86% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/code_row.rs:325-348` function `find_by_address` | `src/store/code_row.rs:353-368` function `symbol_row_exists` | 89.68% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/repo_marker.rs:94-96` function `last_head` | `src/store/repo_marker.rs:120-122` function `root_path` | 89.51% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/retrieval_log.rs:147-158` function `prefix_matches` | `src/store/retrieval_log.rs:163-181` function `distinct_prefix_matches` | 89.37% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/fts_memory.rs:40-58` function `search_memory` | `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | 89.24% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/rebuild_copy_code.rs:28-58` function `copy_code_index_tables` | `src/store/rebuild_copy_code.rs:92-129` function `copy_code_markers` | 89.17% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/indexed_files.rs:22-28` function `delete_for_repo` | `src/store/indexed_files.rs:48-58` function `list_for_repo` | 89.12% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/documents.rs:83-100` function `get_document` | `src/store/documents.rs:108-111` function `delete_document` | 89.03% | parallel document fetch helpers over twin lookups |
| `src/store/fts_memory.rs:63-73` function `search_memory_relaxed` | `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | 89.02% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/code_feedback.rs:75-84` function `upsert_used` | `src/store/code_feedback.rs:90-99` function `upsert_irrelevant` | 88.81% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/orm.rs:41-51` function `query_all` | `src/store/orm.rs:71-79` function `query_row` | 88.77% | shared preparation and binding with distinct collection, required-row and optional-row contracts |
| `src/store/code_ref.rs:144-165` function `for_rel_live` | `src/store/code_ref.rs:168-195` function `for_memory` | 88.71% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/documents.rs:237-250` function `document_id_in_source` | `src/store/documents.rs:255-268` function `document_ids_for_repo_path` | 88.71% | parallel document fetch helpers over twin lookups |
| `src/store/orm.rs:54-60` function `query_one` | `src/store/orm.rs:71-79` function `query_row` | 88.61% | shared preparation and binding with distinct collection, required-row and optional-row contracts |
| `src/store/fts_memory.rs:82-100` function `search_memory_subtokens` | `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | 88.50% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/documents.rs:208-216` function `get_document_path` | `src/store/documents.rs:224-232` function `document_ids_for_source` | 88.43% | parallel document fetch helpers over twin lookups |
| `src/store/edges.rs:395-411` function `memory_ids_referencing_file` | `src/store/edges.rs:416-434` function `src_ids_for_dst_ids` | 88.41% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/feedback.rs:26-34` function `upsert_used` | `src/store/feedback.rs:39-47` function `upsert_irrelevant` | 88.18% | parallel used-vs-irrelevant counters over twin feedback tables |
| `src/store/memory_meta.rs:152-161` function `ids_matching_kind` | `src/store/memory_meta.rs:302-330` function `keeper_stats` | 87.91% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/repo_marker.rs:75-77` function `last_mined_commit` | `src/store/repo_marker.rs:120-122` function `root_path` | 87.85% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/prune_apply.rs:39-49` function `count_orphan_memory_edges` | `src/store/prune_apply.rs:100-108` function `delete_orphan_memory_edges` | 87.84% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/memory_meta.rs:67-91` function `fetch_rows` | `src/store/memory_meta.rs:118-146` function `attach_references` | 87.58% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/code_row.rs:166-173` function `upsert_repo_root` | `src/store/code_row.rs:183-193` function `upsert_last_indexed` | 87.49% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/repo_marker.rs:126-134` function `exists` | `src/store/repo_marker.rs:138-146` function `set_archived` | 87.45% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/code_row.rs:147-157` function `upsert_indexed_file` | `src/store/code_row.rs:183-193` function `upsert_last_indexed` | 87.33% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/indexed_files.rs:22-28` function `delete_for_repo` | `src/store/indexed_files.rs:62-71` function `delete_one` | 87.32% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/orm.rs:63-69` function `query_optional` | `src/store/orm.rs:71-79` function `query_row` | 87.27% | shared preparation and binding with distinct collection, required-row and optional-row contracts |
| `src/store/repo_marker.rs:101-103` function `archived` | `src/store/repo_marker.rs:126-134` function `exists` | 87.19% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/repo_marker.rs:120-122` function `root_path` | `src/store/repo_marker.rs:126-134` function `exists` | 86.97% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/repo_marker.rs:94-96` function `last_head` | `src/store/repo_marker.rs:126-134` function `exists` | 86.97% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sources.rs:230-247` function `touch_file` | `src/store/sources.rs:252-262` function `mark_deleted` | 86.96% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/sources.rs:81-84` function `delete` | `src/store/sources.rs:252-262` function `mark_deleted` | 86.92% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/eval_runs.rs:182-190` function `get` | `src/store/eval_runs.rs:218-220` function `set_applied` | 86.85% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/eval_runs.rs:182-190` function `get` | `src/store/eval_runs.rs:224-226` function `set_discarded` | 86.85% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/indexed_files.rs:33-43` function `blob_oid_for` | `src/store/indexed_files.rs:62-71` function `delete_one` | 86.82% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | `src/store/fts_memory.rs:135-184` function `run_memory_match` | 86.74% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/edges.rs:206-221` function `outgoing` | `src/store/edges.rs:416-434` function `src_ids_for_dst_ids` | 86.72% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/rebuild_copy.rs:69-76` function `old_table_exists` | `src/store/rebuild_copy.rs:81-88` function `old_column_exists` | 86.68% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/code_graph_nodes.rs:243-281` function `top_symbols` | `src/store/code_graph_nodes.rs:295-311` function `citing_memories` | 86.65% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/documents.rs:83-100` function `get_document` | `src/store/documents.rs:224-232` function `document_ids_for_source` | 86.54% | parallel document fetch helpers over twin lookups |
| `src/store/bandit_arms.rs:37-56` function `seed` | `src/store/bandit_arms.rs:72-88` function `load` | 86.53% | distinct seed writes and arm-state reads; statement execution is shared by the ORM bridge |
| `src/store/code_sync.rs:140-151` function `cursor` | `src/store/code_sync.rs:154-157` function `set_cursor` | 86.44% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/feedback.rs:87-100` function `used_query_ids` | `src/store/feedback.rs:125-152` function `used_events_for_golden` | 86.42% | parallel used-vs-irrelevant counters over twin feedback tables |
| `src/store/repo_marker.rs:75-77` function `last_mined_commit` | `src/store/repo_marker.rs:126-134` function `exists` | 86.20% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/memory_meta.rs:67-91` function `fetch_rows` | `src/store/memory_meta.rs:95-112` function `attach_tags` | 86.11% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/documents.rs:108-111` function `delete_document` | `src/store/documents.rs:208-216` function `get_document_path` | 86.07% | parallel document fetch helpers over twin lookups |
| `src/store/memory_meta.rs:53-62` function `fetch_meta` | `src/store/memory_meta.rs:67-91` function `fetch_rows` | 86.05% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/rebuild_copy_history.rs:18-52` function `copy_history_tables` | `src/store/rebuild_copy_history.rs:56-81` function `copy_sync_tables` | 86.05% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/edges.rs:206-221` function `outgoing` | `src/store/edges.rs:258-261` function `delete_outgoing` | 85.97% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/edges.rs:349-370` function `memory_direct_relation_edges` | `src/store/edges.rs:395-411` function `memory_ids_referencing_file` | 85.91% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/indexed_files.rs:33-43` function `blob_oid_for` | `src/store/indexed_files.rs:48-58` function `list_for_repo` | 85.88% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/code_row.rs:261-271` function `parent_snippets` | `src/store/code_row.rs:373-382` function `parent_identity` | 85.69% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/doctor_probes.rs:23-32` function `live_memory_hashes` | `src/store/doctor_probes.rs:48-51` function `live_memory_count` | 85.63% | distinct live-memory hash and count projections over the shared live scope |
| `src/store/code_row.rs:122-141` function `purge_file_symbols` | `src/store/code_row.rs:147-157` function `upsert_indexed_file` | 85.60% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/edges.rs:144-153` function `insert_at` | `src/store/edges.rs:163-172` function `insert_weighted` | 85.55% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/rebuild_copy_documents.rs:34-40` function `copy_source_files` | `src/store/rebuild_copy_documents.rs:43-49` function `copy_documents` | 85.48% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/rebuild_copy_documents.rs:43-49` function `copy_documents` | `src/store/rebuild_copy_documents.rs:53-59` function `copy_document_chunks` | 85.48% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/rebuild_copy_documents.rs:43-49` function `copy_documents` | `src/store/rebuild_copy_documents.rs:63-69` function `copy_document_fts` | 85.48% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/sources.rs:265-271` function `get_file` | `src/store/sources.rs:278-287` function `list_files_by_source` | 85.45% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/index_runs.rs:168-170` function `clamp` | `src/store/index_runs.rs:174-176` function `unsigned` | 85.44% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/query_expansions.rs:35-46` function `insert` | `src/store/query_expansions.rs:93-99` function `count` | 85.44% | distinct expansion insertion and count projection; statement execution is shared by the ORM bridge |
| `src/store/documents.rs:208-216` function `get_document_path` | `src/store/documents.rs:237-250` function `document_id_in_source` | 85.43% | parallel document fetch helpers over twin lookups |
| `src/store/code_graph_nodes.rs:215-219` function `fetch_node` | `src/store/code_graph_nodes.rs:295-311` function `citing_memories` | 85.42% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/repo_marker.rs:53-71` function `read_for_lazy_reindex` | `src/store/repo_marker.rs:138-146` function `set_archived` | 85.40% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/prune_apply.rs:146-175` function `drop_dangling_edges` | `src/store/prune_apply.rs:183-195` function `drop_orphan_code_refs` | 85.35% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/repo_marker.rs:101-103` function `archived` | `src/store/repo_marker.rs:138-146` function `set_archived` | 85.23% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/memory_signals.rs:13-23` function `update_rank_scores` | `src/store/memory_signals.rs:29-45` function `bump_access` | 85.18% | rank batch updates and access-counter updates have different values and write semantics |
| `src/store/edges.rs:321-329` function `delete_co_changed_for_repo` | `src/store/edges.rs:334-343` function `delete_imports_from` | 85.17% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/code_row.rs:78-80` function `stamp_repo_format` | `src/store/code_row.rs:386-394` function `count_for_repo` | 85.08% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/rebuild_copy_code.rs:67-82` function `copy_mined_edges` | `src/store/rebuild_copy_code.rs:135-149` function `copy_code_virtual_tables` | 85.06% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/memory_row.rs:364-370` function `live_ids` | `src/store/memory_row.rs:374-380` function `live_bodies` | 85.04% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |

### `src/config/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/config/paths.rs:89-91` method `sources_file` | `src/config/paths.rs:95-97` method `sources_lock_file` | 93.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/file.rs:184-197` method `apply` | `src/config/file.rs:236-249` method `apply` | 92.58% | the canonical parallel `Config`-section `apply()` case — each merges a different fixed field set, identical in shape because the sections are deliberately symmetric |
| `src/config/env.rs:72-77` function `env_pair` | `src/config/env.rs:82-87` function `env_triple` | 92.22% | different fixed sets of environment fields use the shared override helper |
| `src/config/env.rs:241-259` method `apply_rank_env` | `src/config/env.rs:263-290` method `apply_prune_env` | 91.53% | different fixed sets of environment fields use the shared override helper |
| `src/config/file.rs:184-197` method `apply` | `src/config/file.rs:287-306` method `apply` | 91.32% | the canonical parallel `Config`-section `apply()` case — each merges a different fixed field set, identical in shape because the sections are deliberately symmetric |
| `src/config/file.rs:236-249` method `apply` | `src/config/file.rs:287-306` method `apply` | 91.32% | the canonical parallel `Config`-section `apply()` case — each merges a different fixed field set, identical in shape because the sections are deliberately symmetric |
| `src/config/paths.rs:95-97` method `sources_lock_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 90.03% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/validate_knobs.rs:38-43` function `check_decay` | `src/config/validate_knobs.rs:49-54` function `check_unit_interval` | 90.40% | section-specific field validation; positive-count bounds and error formatting are shared. Moved out of `validate.rs` when that file hit the 300-line ceiling |
| `src/config/paths.rs:36-38` method `memories_dir` | `src/config/paths.rs:46-48` method `index_dir` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:75-77` method `auth_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:83-85` method `allowlist_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:89-91` method `sources_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:75-77` method `auth_file` | `src/config/paths.rs:83-85` method `allowlist_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:75-77` method `auth_file` | `src/config/paths.rs:89-91` method `sources_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:83-85` method `allowlist_file` | `src/config/paths.rs:89-91` method `sources_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/env.rs:17-28` function `env_parse` | `src/config/env.rs:72-77` function `env_pair` | 87.53% | different fixed sets of environment fields use the shared override helper |
| `src/config/env.rs:17-28` function `env_parse` | `src/config/env.rs:82-87` function `env_triple` | 86.91% | different fixed sets of environment fields use the shared override helper |
| `src/config/env.rs:217-222` method `apply_git_env` | `src/config/env.rs:293-300` method `apply_reinforce_env` | 86.26% | different fixed sets of environment fields use the shared override helper |
| `src/config/validate_knobs.rs:11-16` function `check_rrf_k` | `src/config/validate_knobs.rs:38-43` function `check_decay` | 87.64% | section-specific field validation; positive-count bounds and error formatting are shared. Moved out of `validate.rs` when that file hit the 300-line ceiling |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:95-97` method `sources_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:75-77` method `auth_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:75-77` method `auth_file` | `src/config/paths.rs:95-97` method `sources_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:83-85` method `allowlist_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:83-85` method `allowlist_file` | `src/config/paths.rs:95-97` method `sources_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:89-91` method `sources_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/validate.rs:233-256` method `check_rank_knobs` | `src/config/validate.rs:259-294` method `check_prune_knobs` | 85.23% | section-specific field validation; positive-count bounds and error formatting are shared |

### `src/serve/routes/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/serve/routes/learning.rs:165-203` function `tune` | `src/serve/routes/learning.rs:210-248` function `bandit` | 96.51% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/graph_nodes.rs:129-138` function `node_detail` | `src/serve/routes/graph_nodes.rs:208-217` function `node_source` | 96.28% | short typed adapters supply distinct graph queries through shared query_response |
| `src/serve/routes/graph_nodes.rs:100-110` function `list` | `src/serve/routes/graph_nodes.rs:114-124` function `snapshot` | 95.60% | short typed adapters supply distinct graph queries through shared query_response |
| `src/serve/routes/jobs.rs:93-103` function `cancel` | `src/serve/routes/jobs.rs:136-143` function `get_one` | 93.44% | distinct event-stream transitions retain their lifecycle rules; payload serialization is shared |
| `src/serve/routes/jobs.rs:93-103` function `cancel` | `src/serve/routes/jobs.rs:154-168` function `events` | 92.17% | distinct event-stream transitions retain their lifecycle rules; payload serialization is shared |
| `src/serve/routes/learning_console.rs:179-204` function `proposal_apply` | `src/serve/routes/learning_console.rs:210-225` function `proposal_discard` | 91.10% | remaining handlers carry distinct containment, job, or mutation policy; simple reads share query_response |
| `src/serve/routes/jobs.rs:136-143` function `get_one` | `src/serve/routes/jobs.rs:154-168` function `events` | 90.97% | distinct event-stream transitions retain their lifecycle rules; payload serialization is shared |
| `src/serve/routes/jobs.rs:286-297` function `next_log` | `src/serve/routes/jobs.rs:302-311` function `try_next_log` | 90.89% | distinct event-stream transitions retain their lifecycle rules; payload serialization is shared |
| `src/serve/routes/code.rs:88-95` function `code_search_get` | `src/serve/routes/code.rs:97-104` function `code_search_post` | 90.19% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/repos_admin.rs:137-161` function `patch_repo` | `src/serve/routes/repos_admin.rs:166-186` function `archive_repo` | 90.15% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/memory_stores.rs:115-139` function `patch` | `src/serve/routes/memory_stores.rs:145-160` function `create` | 89.82% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/repos_admin.rs:166-186` function `archive_repo` | `src/serve/routes/repos_admin.rs:209-229` function `disconnect_repo` | 89.77% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning_console.rs:110-112` function `summary` | `src/serve/routes/learning_console.rs:157-164` function `proposals` | 89.36% | remaining handlers carry distinct containment, job, or mutation policy; simple reads share query_response |
| `src/serve/routes/find.rs:49-56` function `find_get` | `src/serve/routes/find.rs:59-66` function `find_post` | 89.32% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/repos_admin.rs:137-161` function `patch_repo` | `src/serve/routes/repos_admin.rs:209-229` function `disconnect_repo` | 89.27% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/search.rs:175-182` function `search_get` | `src/serve/routes/search.rs:185-192` function `search_post` | 88.22% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/graph_nodes.rs:129-138` function `node_detail` | `src/serve/routes/graph_nodes.rs:143-153` function `node_neighbors` | 87.65% | short typed adapters supply distinct graph queries through shared query_response |
| `src/serve/routes/graph_nodes.rs:143-153` function `node_neighbors` | `src/serve/routes/graph_nodes.rs:208-217` function `node_source` | 87.23% | short typed adapters supply distinct graph queries through shared query_response |
| `src/serve/routes/learning_console.rs:124-129` function `evals` | `src/serve/routes/learning_console.rs:238-243` function `expansions` | 86.86% | remaining handlers carry distinct containment, job, or mutation policy; simple reads share query_response |
| `src/serve/routes/graph_nodes.rs:100-110` function `list` | `src/serve/routes/graph_nodes.rs:143-153` function `node_neighbors` | 86.40% | short typed adapters supply distinct graph queries through shared query_response |
| `src/serve/routes/graph_nodes.rs:114-124` function `snapshot` | `src/serve/routes/graph_nodes.rs:143-153` function `node_neighbors` | 86.40% | short typed adapters supply distinct graph queries through shared query_response |
| `src/serve/routes/memory_stores.rs:79-88` function `list` | `src/serve/routes/memory_stores.rs:92-101` function `get_one` | 86.31% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/repos_admin.rs:93-118` function `connect_repo` | `src/serve/routes/repos_admin.rs:137-161` function `patch_repo` | 85.92% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/jobs.rs:93-103` function `cancel` | `src/serve/routes/jobs.rs:125-132` function `list` | 85.37% | distinct event-stream transitions retain their lifecycle rules; payload serialization is shared |
| `src/serve/routes/hooks.rs:82-98` function `list_hooks` | `src/serve/routes/hooks.rs:179-214` function `set_hook` | 85.33% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/search.rs:196-212` function `execute` | `src/serve/routes/search.rs:309-322` function `suggest` | 85.04% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |

### `src/domains/sync/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/sync/code.rs:64-70` function `run_code_push` | `src/domains/sync/code.rs:76-82` function `run_code_push_if_moved` | 94.93% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/manual.rs:79-87` function `push_only` | `src/domains/sync/manual.rs:93-109` function `pull_only` | 88.49% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/client.rs:106-116` function `device_code` | `src/domains/sync/client.rs:119-133` function `poll_token` | 88.18% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/manual.rs:62-73` function `run_all` | `src/domains/sync/manual.rs:113-134` function `push_then_code` | 87.50% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/client_code.rs:15-30` function `fetch_code_manifest` | `src/domains/sync/client_code.rs:36-51` function `push_code_import` | 87.06% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/auth_file.rs:106-125` method `load` | `src/domains/sync/auth_file.rs:139-148` method `load_usable` | 86.92% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/auth_file.rs:179-182` method `clear` | `src/domains/sync/auth_file.rs:189-191` function `clear_stale_allowlist` | 86.75% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/client.rs:136-152` function `pull_changes` | `src/domains/sync/client.rs:246-256` function `fetch_manifest` | 86.36% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/manual.rs:62-73` function `run_all` | `src/domains/sync/manual.rs:79-87` function `push_only` | 86.35% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/client.rs:119-133` function `poll_token` | `src/domains/sync/client.rs:246-256` function `fetch_manifest` | 85.87% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/client.rs:195-207` function `ws_ticket` | `src/domains/sync/client.rs:246-256` function `fetch_manifest` | 85.24% | parallel push/pull halves of the same exchange, symmetric by design |

### `src/serve/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/serve/routes.rs:260-270` function `guard_mutating` | `src/serve/routes.rs:277-282` function `guard_job` | 93.07% | parallel request-plumbing helpers over twin shapes |
| `src/serve/envelope.rs:68-76` method `unauthorized` | `src/serve/envelope.rs:115-123` method `confirmation_required` | 90.50% | parallel response-envelope constructors, one per response shape |
| `src/serve/envelope.rs:68-76` method `unauthorized` | `src/serve/envelope.rs:98-106` method `read_only` | 90.50% | parallel response-envelope constructors, one per response shape |
| `src/serve/router.rs:59-68` function `forbidden_response` | `src/serve/router.rs:72-77` function `unauthorized_response` | 89.56% | parallel request-plumbing helpers over twin shapes |
| `src/serve/envelope.rs:68-76` method `unauthorized` | `src/serve/envelope.rs:82-93` method `busy` | 89.25% | parallel response-envelope constructors, one per response shape |
| `src/serve/envelope.rs:98-106` method `read_only` | `src/serve/envelope.rs:115-123` method `confirmation_required` | 89.11% | parallel response-envelope constructors, one per response shape |
| `src/serve/envelope.rs:82-93` method `busy` | `src/serve/envelope.rs:115-123` method `confirmation_required` | 87.47% | parallel response-envelope constructors, one per response shape |
| `src/serve/envelope.rs:82-93` method `busy` | `src/serve/envelope.rs:98-106` method `read_only` | 87.47% | parallel response-envelope constructors, one per response shape |
| `src/serve/routes.rs:296-305` function `respond` | `src/serve/routes.rs:310-315` function `accepted` | 86.59% | parallel request-plumbing helpers over twin shapes |

### `src/serve/routes/memories/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/serve/routes/memories/edit.rs:84-99` function `restore` | `src/serve/routes/memories/edit.rs:105-120` function `refresh_refs` | 94.28% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |
| `src/serve/routes/memories/search.rs:84-99` function `run_search` | `src/serve/routes/memories/search.rs:101-114` function `run_context` | 92.13% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |
| `src/serve/routes/memories/search.rs:38-45` function `memories_search_get` | `src/serve/routes/memories/search.rs:56-63` function `context_get` | 90.92% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |
| `src/serve/routes/memories/search.rs:47-54` function `memories_search_post` | `src/serve/routes/memories/search.rs:65-72` function `context_post` | 90.92% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |
| `src/serve/routes/memories/search.rs:38-45` function `memories_search_get` | `src/serve/routes/memories/search.rs:47-54` function `memories_search_post` | 89.89% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |
| `src/serve/routes/memories/write.rs:53-73` function `save` | `src/serve/routes/memories/write.rs:110-128` function `feedback` | 89.53% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |
| `src/serve/routes/memories/search.rs:56-63` function `context_get` | `src/serve/routes/memories/search.rs:65-72` function `context_post` | 89.44% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |
| `src/serve/routes/memories/edit.rs:60-79` function `update` | `src/serve/routes/memories/edit.rs:84-99` function `restore` | 88.31% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |
| `src/serve/routes/memories/edit.rs:60-79` function `update` | `src/serve/routes/memories/edit.rs:105-120` function `refresh_refs` | 88.15% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |

### `src/domains/integrations/setup/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/integrations/setup/apply.rs:86-96` function `git_hooks` | `src/domains/integrations/setup/apply.rs:99-116` function `index_code` | 89.66% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/plan.rs:63-75` function `data_dir` | `src/domains/integrations/setup/plan.rs:213-225` function `reinforce` | 87.43% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:61-81` function `agent_host` | `src/domains/integrations/setup/apply.rs:86-96` function `git_hooks` | 86.88% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:61-81` function `agent_host` | `src/domains/integrations/setup/apply.rs:99-116` function `index_code` | 86.88% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:54-58` function `data_dir` | `src/domains/integrations/setup/apply.rs:119-129` function `reinforce` | 86.78% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:34-50` function `one` | `src/domains/integrations/setup/apply.rs:99-116` function `index_code` | 86.55% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/detect.rs:207-219` function `hosts_present` | `src/domains/integrations/setup/detect.rs:226-232` function `hosts_installed` | 86.00% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:99-116` function `index_code` | `src/domains/integrations/setup/apply.rs:119-129` function `reinforce` | 85.27% | parallel per-integration setup arms, one per supported agent target |

### `src/domains/retrieval/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/retrieval/score.rs:156-166` function `min_max_normalize` | `src/domains/retrieval/score.rs:183-189` function `max_normalize` | 91.94% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/pipeline.rs:178-206` function `record_telemetry` | `src/domains/retrieval/pipeline.rs:256-267` function `record_query` | 91.84% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/fuse.rs:54-56` function `rrf_multi` | `src/domains/retrieval/fuse.rs:65-67` function `rrf_multi_weighted` | 90.21% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/scope.rs:70-75` method `window` | `src/domains/retrieval/scope.rs:126-128` method `window` | 89.43% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/suggest.rs:96-107` function `expansions` | `src/domains/retrieval/suggest.rs:116-127` function `recent` | 89.21% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/router.rs:254-271` function `route_vector_only` | `src/domains/retrieval/router.rs:340-358` function `route_lexical` | 88.14% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/fuse.rs:32-34` function `rrf` | `src/domains/retrieval/fuse.rs:40-42` function `rrf_k` | 86.10% | parallel scorers/normalizers over twin candidate shapes |

### `src/serve/jobs/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/serve/jobs/worker.rs:31-45` function `spawn_job` | `src/serve/jobs/worker.rs:73-94` function `spawn_job_for` | 92.05% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/registry.rs:320-327` function `active_in` | `src/serve/jobs/registry.rs:332-340` function `refuse_active` | 90.33% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/worker.rs:31-45` function `spawn_job` | `src/serve/jobs/worker.rs:53-64` function `spawn_job_with_id` | 88.65% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/registry.rs:287-289` method `subscribe` | `src/serve/jobs/registry.rs:293-298` method `subscribe_progress` | 88.20% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/registry.rs:210-212` method `subscribe_log` | `src/serve/jobs/registry.rs:287-289` method `subscribe` | 86.61% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/events.rs:30-37` method `new` | `src/serve/jobs/events.rs:57-64` method `new` | 85.35% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/events.rs:30-37` method `new` | `src/serve/jobs/events.rs:80-82` method `new` | 85.35% | parallel job-registry / worker arms, one per job kind |

### `src/domains/graph/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/graph/memory_rank.rs:78-89` function `push_direct_edges` | `src/domains/graph/memory_rank.rs:94-106` function `push_co_citation_edges` | 95.58% | direct-relation vs. co-citation edge pushes — twin PageRank edge loaders over twin queries |
| `src/domains/graph/imports.rs:309-324` function `python_imports` | `src/domains/graph/imports.rs:328-351` function `go_imports` | 92.74% | Python and Go line parsers handle different syntax; import collection is shared |
| `src/domains/graph/graph_nodes.rs:300-314` function `fetch_top_symbols` | `src/domains/graph/graph_nodes.rs:319-328` function `fetch_cited_by` | 88.98% | parallel graph-page builders / row-to-DTO mappers |
| `src/domains/graph/doc_link.rs:101-139` function `resolve_markdown_links` | `src/domains/graph/doc_link.rs:157-176` function `resolve_document_id` | 88.71% | parallel graph-page builders / row-to-DTO mappers |
| `src/domains/graph/query.rs:39-48` function `build_code_graph` | `src/domains/graph/query.rs:54-66` function `build_graph_page` | 85.67% | parallel graph-page builders / row-to-DTO mappers |
| `src/domains/graph/doc_link.rs:22-46` function `derive_after_document` | `src/domains/graph/doc_link.rs:101-139` function `resolve_markdown_links` | 85.28% | parallel graph-page builders / row-to-DTO mappers |

### `src/domains/memories/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/memories/save.rs:358-367` function `near_duplicate` | `src/domains/memories/save.rs:372-391` function `near_duplicate_inner` | 88.69% | parallel memory command handlers over twin operations |
| `src/domains/memories/update.rs:282-297` function `mirror_row` | `src/domains/memories/update.rs:300-315` function `append_local_upsert` | 85.34% | parallel memory command handlers over twin operations |
| `src/domains/memories/list.rs:77-83` method `from` | `src/domains/memories/list.rs:115-128` method `from` | 85.16% | parallel memory command handlers over twin operations |

### `src/utilities/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/utilities/id_list.rs:30-40` function `parse_id_csv` | `src/utilities/id_list.rs:48-65` function `parse_symbol_id_csv` | 87.88% | parallel CSV-parse-into-Vec helpers for two id kinds |
| `src/utilities/fetch.rs:47-64` function `download` | `src/utilities/fetch.rs:67-93` function `final_url` | 86.19% | parallel fetch helpers over twin response shapes |
| `src/utilities/fetch.rs:95-124` function `exchange_curl` | `src/utilities/fetch.rs:126-171` function `exchange_wget` | 85.63% | parallel fetch helpers over twin response shapes |

### `src/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/serve.rs:145-151` method `swap_conn` | `src/serve.rs:162-171` method `reload_cfg` | 88.13% | trivial parallel getter methods on `AppState` |
| `src/serve.rs:194-196` method `repo` | `src/serve.rs:219-221` method `embed_cmd` | 87.31% | trivial parallel getter methods on `AppState` |

### `src/cli/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/cli/capture.rs:85-112` function `run_sources` | `src/cli/capture.rs:135-149` function `run_session` | 86.69% | parallel CLI command helpers over twin inputs |
| `src/cli/sync_render.rs:73-90` function `emit_daemon_status` | `src/cli/sync_render.rs:151-172` function `emit_verify` | 86.07% | parallel CLI command helpers over twin inputs |

### `src/cli/output/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/cli/output/prune.rs:36-43` function `write_list` | `src/cli/output/prune.rs:49-60` function `write_rows` | 93.08% | parallel render helpers, one per output shape |
| `src/cli/output/graph.rs:33-37` function `write_dot` | `src/cli/output/graph.rs:40-44` function `write_html` | 89.98% | parallel render helpers, one per output shape |

### `src/domains/code/ast/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/code/ast/extractor.rs:146-156` function `python_patterns` | `src/domains/code/ast/extractor.rs:158-169` function `go_patterns` | 91.82% | distinct Python and Go symbol pattern tables; language cache dispatch is shared |
| `src/domains/code/ast/pattern_cache.rs:31-40` function `cached` | `src/domains/code/ast/pattern_cache.rs:44-55` function `compile_patterns` | 85.31% | cache-hit vs. compile-miss halves of the same lookup |

### `src/domains/sync/exchange/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/sync/exchange/import_state.rs:58-64` function `id_collision` | `src/domains/sync/exchange/import_state.rs:68-74` function `id_collision_for_test` | 95.78% | parallel import arms, one per exchanged record kind |
| `src/domains/sync/exchange/import_rules.rs:93-138` function `apply_restore` | `src/domains/sync/exchange/import_rules.rs:140-204` function `apply_upsert` | 86.56% | parallel import arms, one per exchanged record kind |

### `src/domains/sync/memory_store/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/sync/memory_store/git.rs:157-167` function `unmerged_paths` | `src/domains/sync/memory_store/git.rs:191-194` function `head` | 86.43% | parallel store-backend operations over twin transports |
| `src/domains/sync/memory_store/git.rs:191-194` function `head` | `src/domains/sync/memory_store/git.rs:203-211` function `push` | 85.99% | parallel store-backend operations over twin transports |

### `src/serve/routes/maint/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/serve/routes/maint/prune.rs:91-112` function `prune_apply` | `src/serve/routes/maint/prune.rs:151-166` function `gc` | 91.96% | parallel maintenance handlers — same extract-validate-respond skeleton per route |
| `src/serve/routes/maint/admin.rs:64-79` function `mine` | `src/serve/routes/maint/admin.rs:86-105` function `hooks_install` | 88.32% | parallel maintenance handlers — same extract-validate-respond skeleton per route |

### `src/domains/code/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/code/git_utils.rs:343-345` function `hook_installed` | `src/domains/code/git_utils.rs:361-364` function `hook_outdated` | 89.13% | parallel code-indexing helpers over twin inputs |

### `src/domains/documents/document/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/documents/document/extract.rs:70-84` function `extract_txt` | `src/domains/documents/document/extract.rs:86-95` function `extract_markdown` | 88.81% | parallel document extraction arms, one per source shape |

### `src/domains/learning/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/learning/learning_proposals.rs:181-189` function `parse_knobs` | `src/domains/learning/learning_proposals.rs:193-200` function `require_knobs` | 87.70% | parallel proposal builders over twin signal sources |

### `src/domains/maintenance/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/maintenance/reembed.rs:300-320` function `reembed_memories` | `src/domains/maintenance/reembed.rs:324-340` function `reembed_code` | 92.83% | parallel maintenance checks, one per checked surface |

### `src/domains/maintenance/doctor/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/maintenance/doctor/checks.rs:62-64` function `warn` | `src/domains/maintenance/doctor/checks.rs:67-69` function `fail` | 92.09% | parallel maintenance checks, one per checked surface |

### `src/domains/retrieval/unified/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/retrieval/unified/fuse_domains.rs:213-226` function `code_hit` | `src/domains/retrieval/unified/fuse_domains.rs:229-248` function `doc_hit` | 89.78% | parallel per-domain fusion arms |

### `src/mcp/`

The twelve read tools are one `#[tool_router]` block, and rmcp 3.4 forces the
shape the detector scores: every tool must be a real method carrying
`#[tool(description = "<literal>")]` with the same
`(&self, Parameters<T>) -> Result<CallToolResult, ErrorData>` signature, and
`#[tool_router]` registers nothing that a `macro_rules!` would generate (memory
`rmcp-3-4-macro-constraints`). Each body is one `read_tool(self, move |c, s| { … })`
call whose closure already differs per tool (core module, `track()` handling,
envelope builder); the `*_data` helpers and the `run_read`/`run_write` pair were
folded away first, which is what the count below excludes. These 31 pairs are the
signature-and-attribute skeleton, not shared logic; the baseline moved from 235
to 266 as the MCP transport added three architecture tools.

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/mcp/tools_read.rs:40-50` method `architecture_scaffold` | `src/mcp/tools_read.rs:57-77` method `architecture_show` | 91.85% | rmcp-forced tool skeleton; the bodies call distinct architecture cores and renderers |
| `src/mcp/tools_read.rs:57-77` method `architecture_show` | `src/mcp/tools_read.rs:84-94` method `architecture_check` | 91.85% | rmcp-forced tool skeleton; the bodies call distinct architecture cores and renderers |
| `src/mcp/tools_read.rs:40-50` method `architecture_scaffold` | `src/mcp/tools_read.rs:84-94` method `architecture_check` | 96.62% | rmcp-forced tool skeleton; the bodies call distinct architecture cores and renderers |
| `src/mcp/tools_read.rs:40-57` method `find` | `src/mcp/tools_read.rs:66-84` method `search` | 93.41% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:66-84` method `search` | `src/mcp/tools_read.rs:114-130` method `context` | 93.41% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:40-57` method `find` | `src/mcp/tools_read.rs:114-130` method `context` | 93.26% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:66-84` method `search` | `src/mcp/tools_read.rs:93-105` method `search_code` | 95.91% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:66-84` method `search` | `src/mcp/tools_read.rs:171-183` method `edges` | 89.72% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:40-57` method `find` | `src/mcp/tools_read.rs:93-105` method `search_code` | 91.60% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:40-57` method `find` | `src/mcp/tools_read.rs:171-183` method `edges` | 89.54% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:93-105` method `search_code` | `src/mcp/tools_read.rs:114-130` method `context` | 91.56% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:114-130` method `context` | `src/mcp/tools_read.rs:171-183` method `edges` | 89.50% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:66-84` method `search` | `src/mcp/tools_read.rs:153-162` method `list` | 91.24% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:66-84` method `search` | `src/mcp/tools_read.rs:211-220` method `recall_status` | 89.88% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:40-57` method `find` | `src/mcp/tools_read.rs:153-162` method `list` | 91.09% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:40-57` method `find` | `src/mcp/tools_read.rs:211-220` method `recall_status` | 89.73% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:114-130` method `context` | `src/mcp/tools_read.rs:153-162` method `list` | 91.06% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:114-130` method `context` | `src/mcp/tools_read.rs:211-220` method `recall_status` | 89.69% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:93-105` method `search_code` | `src/mcp/tools_read.rs:171-183` method `edges` | 88.07% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:171-183` method `edges` | `src/mcp/tools_read.rs:192-202` method `repos` | 89.92% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:93-105` method `search_code` | `src/mcp/tools_read.rs:153-162` method `list` | 89.54% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:153-162` method `list` | `src/mcp/tools_read.rs:171-183` method `edges` | 88.66% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:93-105` method `search_code` | `src/mcp/tools_read.rs:211-220` method `recall_status` | 88.43% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:171-183` method `edges` | `src/mcp/tools_read.rs:211-220` method `recall_status` | 87.42% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:153-162` method `list` | `src/mcp/tools_read.rs:192-202` method `repos` | 87.76% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:192-202` method `repos` | `src/mcp/tools_read.rs:211-220` method `recall_status` | 86.53% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:153-162` method `list` | `src/mcp/tools_read.rs:211-220` method `recall_status` | 90.56% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:139-144` method `show` | `src/mcp/tools_read.rs:171-183` method `edges` | 89.92% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:139-144` method `show` | `src/mcp/tools_read.rs:192-202` method `repos` | 90.63% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:139-144` method `show` | `src/mcp/tools_read.rs:153-162` method `list` | 89.95% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:139-144` method `show` | `src/mcp/tools_read.rs:211-220` method `recall_status` | 86.53% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/config/validate_knobs.rs:20-25` function `check_graph_hops` | `src/config/validate_knobs.rs:38-43` function `check_decay` | 86.09% | one bound per knob; the shapes rhyme because each is `if out of range { Err(msg) } else { Ok(()) }`, and each message names its own range |
| `src/config/validate_knobs.rs:30-35` function `check_positive_count` | `src/config/validate_knobs.rs:38-43` function `check_decay` | 86.09% | same: one bound per knob, each with its own message |
| `src/config/validate_knobs.rs:20-25` function `check_graph_hops` | `src/config/validate_knobs.rs:30-35` function `check_positive_count` | 85.79% | same: one bound per knob, each with its own message |
| `src/store/activity.rs:136-152` function `insert` | `src/store/activity.rs:210-221` function `newest_id` | 87.86% | per-table query projections: one INSERT and one ordered SELECT over the same builder idiom; merging them would fuse a write with a read |
| `src/store/activity_rollups.rs:43-61` function `rollups` | `src/store/activity_rollups.rs:90-104` function `sample_durations` | 85.14% | the aggregate and the sample it draws from: different columns, different ordering, different decode |
| `src/store/activity_rollups.rs:64-76` function `distinct_commands` | `src/store/activity_rollups.rs:90-104` function `sample_durations` | 86.81% | two projections over the same filtered window; the shared half is already `activity::apply_filter` |
| `src/serve/routes/activity_stream.rs:151-172` function `stream` | `src/serve/routes/activity_stream.rs:200-212` function `emit` | 87.34% | short route adapters: the unfold step and the event it yields, one owning the loop and the other the encoding |

## Continuing the cleanup

The remaining matches include distinct per-language parsers, per-table query
projections, short route adapters, and configuration sections with different
fields. Their structural similarity does not by itself establish that their
behavior can be merged. The rationale column records the distinction to inspect
before extracting another helper; it is not an exemption from the gate.

Shared query execution, AST traversal, environment overrides, validation errors,
HTTP query responses and job-event serialization have already been extracted.
Further cleanup should preserve each caller's ordering, errors and mutation
policy, then lower both this inventory and `dup-baseline.txt` in the same change.
