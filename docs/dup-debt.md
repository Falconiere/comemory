# Duplication debt baseline (`scripts/dup-check.sh`)

Status: documented baseline, tracked by a count ratchet against a **pinned**
`similarity-rs` · Owner: whoever burns a pair down next

**242 near-duplicate function/method pairs at threshold 0.85**, measured with
**`similarity-rs 0.5.0`** over the 411 production `.rs` files under `src/`, with
pairs found in 92 of them. That number and the tool that produced it are
recorded together, here and in `dup-baseline.txt`, because either one alone is
meaningless.

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
  — 411 files. Colocated tests are excluded because CORE excludes tests from
  copy-paste detection: repeated setup there is not the duplication worth
  chasing.
- **Result at fix time:** 242 pairs over those 411 files. The count is
  deterministic: three consecutive runs of the pinned build returned 242 every
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

- The baseline count (`242`) lives in `dup-baseline.txt`, a single tracked
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

## The 242 pairs by area

Counts sum to the recorded baseline; if they ever stop matching, one of the two
is stale.

| Area | Pairs |
| --- | --- |
| `src/store/` | 70 |
| `src/serve/routes/` | 37 |
| `src/config/` | 36 |
| `src/domains/code/ast/` | 12 |
| `src/domains/graph/` | 12 |
| `src/domains/sync/` | 11 |
| `src/serve/` | 9 |
| `src/serve/routes/memories/` | 9 |
| `src/domains/integrations/setup/` | 8 |
| `src/domains/retrieval/` | 7 |
| `src/serve/jobs/` | 7 |
| `src/domains/memories/` | 3 |
| `src/utilities/` | 3 |
| `src/` | 2 |
| `src/cli/` | 2 |
| `src/cli/output/` | 2 |
| `src/domains/sync/exchange/` | 2 |
| `src/domains/sync/memory_store/` | 2 |
| `src/serve/routes/maint/` | 2 |
| `src/domains/code/` | 1 |
| `src/domains/documents/document/` | 1 |
| `src/domains/learning/` | 1 |
| `src/domains/maintenance/` | 1 |
| `src/domains/maintenance/doctor/` | 1 |
| `src/domains/retrieval/unified/` | 1 |
| **Total** | **242** |

## The 242 pairs, grouped by area

Grouped by the directory of the first function in each pair. Similarity is
`similarity-rs`'s APTED-based score at `--threshold 0.85`; line ranges are the
function/method body span at scan time and will drift with unrelated edits to
the same file — harmless, since the ratchet re-scans fresh each run and pins no
line numbers.

### `src/store/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/store/fts_memory.rs:40-58` function `search_memory` | `src/store/fts_memory.rs:82-100` function `search_memory_subtokens` | 98.78% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/fts_memory.rs:40-58` function `search_memory` | `src/store/fts_memory.rs:63-73` function `search_memory_relaxed` | 98.09% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/sync_log.rs:131-159` function `entries_since` | `src/store/sync_log.rs:162-191` function `local_entries_since` | 97.96% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/sync_log.rs:34-41` method `parse` | `src/store/sync_log.rs:64-70` method `parse` | 97.95% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/fts_memory.rs:63-73` function `search_memory_relaxed` | `src/store/fts_memory.rs:82-100` function `search_memory_subtokens` | 97.35% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/sync_log.rs:25-31` method `as_str` | `src/store/sync_log.rs:56-61` method `as_str` | 96.75% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/vector.rs:37-45` function `dim_memory` | `src/store/vector.rs:48-56` function `dim_code` | 94.90% | deliberate per-table repetition across the parallel vector tables |
| `src/store/edges.rs:249-255` function `delete_outgoing` | `src/store/edges.rs:259-265` function `delete_touching` | 93.65% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/sources.rs:112-122` function `row_from_sql` | `src/store/sources.rs:310-324` function `file_row_from_sql` | 93.26% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/eval_runs.rs:216-218` function `set_applied` | `src/store/eval_runs.rs:222-224` function `set_discarded` | 92.20% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/documents.rs:73-82` function `get_document` | `src/store/documents.rs:185-195` function `get_document_path` | 92.10% | parallel document fetch helpers over twin lookups |
| `src/store/sources.rs:95-104` function `get` | `src/store/sources.rs:253-257` function `get_file` | 92.10% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/repo_marker.rs:72-74` function `last_mined_commit` | `src/store/repo_marker.rs:91-93` function `last_head` | 92.00% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/code_feedback.rs:39-51` function `own_identity` | `src/store/code_feedback.rs:55-64` function `parent_identity` | 91.24% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/sync_state.rs:56-63` function `set_pushed` | `src/store/sync_state.rs:66-73` function `set_pulled` | 90.88% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/sources.rs:59-72` function `upsert` | `src/store/sources.rs:203-221` function `upsert_file` | 90.50% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/vector.rs:59-67` function `insert_memory` | `src/store/vector.rs:178-186` function `insert_code` | 90.23% | deliberate per-table repetition across the parallel vector tables |
| `src/store/rebuild_copy_documents.rs:53-59` function `copy_document_chunks` | `src/store/rebuild_copy_documents.rs:63-69` function `copy_document_fts` | 89.86% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/code_ref.rs:40-53` function `materialize` | `src/store/code_ref.rs:79-93` function `upsert` | 89.56% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/memory_meta.rs:95-115` function `attach_tags` | `src/store/memory_meta.rs:121-156` function `attach_references` | 89.52% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/repo_marker.rs:91-93` function `last_head` | `src/store/repo_marker.rs:114-116` function `root_path` | 89.50% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/retrieval_log.rs:140-151` function `prefix_matches` | `src/store/retrieval_log.rs:156-174` function `distinct_prefix_matches` | 89.37% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/vector.rs:59-67` function `insert_memory` | `src/store/vector.rs:76-82` function `replace_memory` | 89.28% | deliberate per-table repetition across the parallel vector tables |
| `src/store/fts_memory.rs:40-58` function `search_memory` | `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | 89.24% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/rebuild_copy_code.rs:28-58` function `copy_code_index_tables` | `src/store/rebuild_copy_code.rs:92-129` function `copy_code_markers` | 89.17% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/vector.rs:178-186` function `insert_code` | `src/store/vector.rs:190-196` function `replace_code` | 89.14% | deliberate per-table repetition across the parallel vector tables |
| `src/store/fts_memory.rs:63-73` function `search_memory_relaxed` | `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | 89.02% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/memory_meta.rs:217-239` function `fetch_extra` | `src/store/memory_meta.rs:266-290` function `rank_signals` | 89.02% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/code_feedback.rs:69-78` function `upsert_used` | `src/store/code_feedback.rs:84-93` function `upsert_irrelevant` | 88.81% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/edges.rs:350-360` function `memory_direct_relation_edges` | `src/store/edges.rs:365-377` function `memory_co_citation_edges` | 88.58% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/fts_memory.rs:82-100` function `search_memory_subtokens` | `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | 88.50% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/documents.rs:159-178` function `get_chunk` | `src/store/documents.rs:185-195` function `get_document_path` | 88.48% | parallel document fetch helpers over twin lookups |
| `src/store/memory_meta.rs:181-189` function `kind_and_body` | `src/store/memory_meta.rs:217-239` function `fetch_extra` | 88.34% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/memory_row.rs:182-198` function `mined_edges` | `src/store/memory_row.rs:204-220` function `restore_mined_edges` | 88.31% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/code_row.rs:324-343` function `find_by_address` | `src/store/code_row.rs:348-361` function `symbol_row_exists` | 88.18% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/feedback.rs:22-30` function `upsert_used` | `src/store/feedback.rs:35-43` function `upsert_irrelevant` | 88.18% | parallel used-vs-irrelevant counters over twin feedback tables |
| `src/store/prune_apply.rs:34-44` function `count_orphan_memory_edges` | `src/store/prune_apply.rs:92-100` function `delete_orphan_memory_edges` | 87.84% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/repo_marker.rs:72-74` function `last_mined_commit` | `src/store/repo_marker.rs:114-116` function `root_path` | 87.83% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/code_row.rs:58-70` function `ensure_repo_format` | `src/store/code_row.rs:77-84` function `stamp_repo_format` | 87.73% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/code_row.rs:167-174` function `upsert_repo_root` | `src/store/code_row.rs:184-194` function `upsert_last_indexed` | 87.49% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/code_graph_nodes.rs:238-267` function `top_symbols` | `src/store/code_graph_nodes.rs:281-297` function `citing_memories` | 87.36% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/code_row.rs:148-158` function `upsert_indexed_file` | `src/store/code_row.rs:184-194` function `upsert_last_indexed` | 87.33% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/code_ref.rs:126-146` function `for_rel_live` | `src/store/code_ref.rs:149-169` function `for_memory` | 86.90% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | `src/store/fts_memory.rs:135-184` function `run_memory_match` | 86.74% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/rebuild_copy.rs:69-76` function `old_table_exists` | `src/store/rebuild_copy.rs:81-88` function `old_column_exists` | 86.68% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/documents.rs:90-93` function `delete_document` | `src/store/documents.rs:185-195` function `get_document_path` | 86.58% | parallel document fetch helpers over twin lookups |
| `src/store/documents.rs:73-82` function `get_document` | `src/store/documents.rs:90-93` function `delete_document` | 86.53% | parallel document fetch helpers over twin lookups |
| `src/store/code_sync.rs:129-140` function `cursor` | `src/store/code_sync.rs:143-146` function `set_cursor` | 86.44% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/feedback.rs:77-90` function `used_query_ids` | `src/store/feedback.rs:115-142` function `used_events_for_golden` | 86.42% | parallel used-vs-irrelevant counters over twin feedback tables |
| `src/store/edges.rs:158-167` function `insert_weighted` | `src/store/edges.rs:173-183` function `current_weight` | 86.15% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/memory_meta.rs:162-176` function `ids_matching_kind` | `src/store/memory_meta.rs:312-338` function `keeper_stats` | 86.15% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/rebuild_copy_history.rs:18-52` function `copy_history_tables` | `src/store/rebuild_copy_history.rs:56-81` function `copy_sync_tables` | 86.05% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/sources.rs:76-79` function `delete` | `src/store/sources.rs:95-104` function `get` | 85.93% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/indexed_files.rs:24-32` function `blob_oid_for` | `src/store/indexed_files.rs:48-54` function `delete_one` | 85.81% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/code_row.rs:126-142` function `purge_file_symbols` | `src/store/code_row.rs:167-174` function `upsert_repo_root` | 85.80% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/memory_row.rs:334-340` function `live_ids` | `src/store/memory_row.rs:344-351` function `live_bodies` | 85.76% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/code_row.rs:199-222` function `insert` | `src/store/code_row.rs:375-382` function `count_for_repo` | 85.58% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/edges.rs:139-148` function `insert_at` | `src/store/edges.rs:158-167` function `insert_weighted` | 85.55% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/vector.rs:132-167` function `knn_memory` | `src/store/vector.rs:207-235` function `knn_code` | 85.52% | deliberate per-table repetition across the parallel vector tables |
| `src/store/rebuild_copy_documents.rs:34-40` function `copy_source_files` | `src/store/rebuild_copy_documents.rs:43-49` function `copy_documents` | 85.48% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/rebuild_copy_documents.rs:43-49` function `copy_documents` | `src/store/rebuild_copy_documents.rs:53-59` function `copy_document_chunks` | 85.48% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/rebuild_copy_documents.rs:43-49` function `copy_documents` | `src/store/rebuild_copy_documents.rs:63-69` function `copy_document_fts` | 85.48% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/index_runs.rs:187-189` function `clamp` | `src/store/index_runs.rs:193-195` function `unsigned` | 85.44% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/code_graph_nodes.rs:210-214` function `fetch_node` | `src/store/code_graph_nodes.rs:281-297` function `citing_memories` | 85.42% | parallel code-index row mappers; each keeps its SQL inline and auditable rather than sharing a table-name-parameterized helper |
| `src/store/edges_retrieval.rs:115-149` function `walk_context_edges` | `src/store/edges_retrieval.rs:216-229` function `direct_reference_edges` | 85.35% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/prune_apply.rs:138-167` function `drop_dangling_edges` | `src/store/prune_apply.rs:175-187` function `drop_orphan_code_refs` | 85.35% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/memory_row.rs:145-160` function `relation_edge_stamps` | `src/store/memory_row.rs:287-321` function `insert_relation_edges` | 85.18% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/retrieval_log.rs:104-122` function `queries_excluding_source` | `src/store/retrieval_log.rs:140-151` function `prefix_matches` | 85.11% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/sync_binding.rs:24-31` function `bind_first` | `src/store/sync_binding.rs:54-70` function `get` | 85.10% | parallel CRUD/row-mapper pairs over twin tables; each keeps its SQL inline and auditable, which is the documented shape of `store/` |
| `src/store/rebuild_copy_code.rs:67-82` function `copy_mined_edges` | `src/store/rebuild_copy_code.rs:135-149` function `copy_code_virtual_tables` | 85.06% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |

### `src/serve/routes/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/serve/routes/graph_nodes.rs:139-153` function `node_detail` | `src/serve/routes/graph_nodes.rs:228-242` function `node_source` | 96.83% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning.rs:165-203` function `tune` | `src/serve/routes/learning.rs:210-248` function `bandit` | 96.51% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/graph_nodes.rs:100-115` function `list` | `src/serve/routes/graph_nodes.rs:119-134` function `snapshot` | 95.91% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning_console.rs:108-118` function `summary` | `src/serve/routes/learning_console.rs:168-178` function `proposals` | 94.91% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/jobs.rs:93-103` function `cancel` | `src/serve/routes/jobs.rs:136-143` function `get_one` | 93.44% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning_console.rs:130-140` function `evals` | `src/serve/routes/learning_console.rs:252-262` function `expansions` | 92.34% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/jobs.rs:93-103` function `cancel` | `src/serve/routes/jobs.rs:154-168` function `events` | 92.17% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning_console.rs:130-140` function `evals` | `src/serve/routes/learning_console.rs:153-164` function `golden_set` | 91.36% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning_console.rs:153-164` function `golden_set` | `src/serve/routes/learning_console.rs:252-262` function `expansions` | 91.36% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning_console.rs:193-218` function `proposal_apply` | `src/serve/routes/learning_console.rs:224-239` function `proposal_discard` | 91.10% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/jobs.rs:136-143` function `get_one` | `src/serve/routes/jobs.rs:154-168` function `events` | 90.97% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/jobs.rs:286-297` function `next_log` | `src/serve/routes/jobs.rs:302-311` function `try_next_log` | 90.89% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/code.rs:88-95` function `code_search_get` | `src/serve/routes/code.rs:97-104` function `code_search_post` | 90.19% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/repos_admin.rs:137-161` function `patch_repo` | `src/serve/routes/repos_admin.rs:166-186` function `archive_repo` | 90.15% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/memory_stores.rs:115-139` function `patch` | `src/serve/routes/memory_stores.rs:145-160` function `create` | 89.82% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/repos_admin.rs:166-186` function `archive_repo` | `src/serve/routes/repos_admin.rs:209-229` function `disconnect_repo` | 89.77% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/find.rs:49-56` function `find_get` | `src/serve/routes/find.rs:59-66` function `find_post` | 89.32% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/repos_admin.rs:137-161` function `patch_repo` | `src/serve/routes/repos_admin.rs:209-229` function `disconnect_repo` | 89.27% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/graph_nodes.rs:139-153` function `node_detail` | `src/serve/routes/graph_nodes.rs:158-173` function `node_neighbors` | 89.04% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/jobs.rs:383-391` function `encode_progress` | `src/serve/routes/jobs.rs:395-403` function `encode_log` | 88.99% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/graph_nodes.rs:100-115` function `list` | `src/serve/routes/graph_nodes.rs:158-173` function `node_neighbors` | 88.83% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/graph_nodes.rs:119-134` function `snapshot` | `src/serve/routes/graph_nodes.rs:158-173` function `node_neighbors` | 88.83% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/graph_nodes.rs:158-173` function `node_neighbors` | `src/serve/routes/graph_nodes.rs:228-242` function `node_source` | 88.79% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/search.rs:175-182` function `search_get` | `src/serve/routes/search.rs:185-192` function `search_post` | 88.22% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/jobs.rs:371-379` function `encode_status` | `src/serve/routes/jobs.rs:383-391` function `encode_progress` | 87.71% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/jobs.rs:371-379` function `encode_status` | `src/serve/routes/jobs.rs:395-403` function `encode_log` | 87.71% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning_console.rs:108-118` function `summary` | `src/serve/routes/learning_console.rs:130-140` function `evals` | 87.48% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning_console.rs:130-140` function `evals` | `src/serve/routes/learning_console.rs:168-178` function `proposals` | 87.34% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning_console.rs:108-118` function `summary` | `src/serve/routes/learning_console.rs:252-262` function `expansions` | 87.25% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning_console.rs:168-178` function `proposals` | `src/serve/routes/learning_console.rs:252-262` function `expansions` | 87.25% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning_console.rs:108-118` function `summary` | `src/serve/routes/learning_console.rs:153-164` function `golden_set` | 86.94% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/learning_console.rs:153-164` function `golden_set` | `src/serve/routes/learning_console.rs:168-178` function `proposals` | 86.94% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/memory_stores.rs:79-88` function `list` | `src/serve/routes/memory_stores.rs:92-101` function `get_one` | 86.31% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/repos_admin.rs:93-118` function `connect_repo` | `src/serve/routes/repos_admin.rs:137-161` function `patch_repo` | 85.92% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/jobs.rs:93-103` function `cancel` | `src/serve/routes/jobs.rs:125-132` function `list` | 85.37% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/hooks.rs:82-98` function `list_hooks` | `src/serve/routes/hooks.rs:179-214` function `set_hook` | 85.33% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/search.rs:196-212` function `execute` | `src/serve/routes/search.rs:309-322` function `suggest` | 85.04% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |

### `src/config/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/config/paths.rs:89-91` method `sources_file` | `src/config/paths.rs:95-97` method `sources_lock_file` | 93.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/env.rs:211-225` method `apply_rank_env` | `src/config/env.rs:229-246` method `apply_prune_env` | 92.98% | parallel per-section environment-override readers; each `apply_*_env` merges a different fixed set of knobs, symmetric by design |
| `src/config/file.rs:184-197` method `apply` | `src/config/file.rs:236-249` method `apply` | 92.58% | the canonical parallel `Config`-section `apply()` case — each merges a different fixed field set, identical in shape because the sections are deliberately symmetric |
| `src/config/env.rs:72-77` function `env_pair` | `src/config/env.rs:82-87` function `env_triple` | 92.22% | parallel per-section environment-override readers; each `apply_*_env` merges a different fixed set of knobs, symmetric by design |
| `src/config/file.rs:184-197` method `apply` | `src/config/file.rs:287-306` method `apply` | 91.32% | the canonical parallel `Config`-section `apply()` case — each merges a different fixed field set, identical in shape because the sections are deliberately symmetric |
| `src/config/file.rs:236-249` method `apply` | `src/config/file.rs:287-306` method `apply` | 91.32% | the canonical parallel `Config`-section `apply()` case — each merges a different fixed field set, identical in shape because the sections are deliberately symmetric |
| `src/config/paths.rs:95-97` method `sources_lock_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 90.03% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/env.rs:115-132` method `apply_indexing_env` | `src/config/env.rs:173-184` method `apply_sync_env` | 89.84% | parallel per-section environment-override readers; each `apply_*_env` merges a different fixed set of knobs, symmetric by design |
| `src/config/validate.rs:216-224` method `check_indexing_knobs` | `src/config/validate.rs:321-329` method `check_reinforce_knobs` | 89.70% | parallel per-section knob validators; each `check_*_knobs` range-checks a different fixed field set |
| `src/config/validate.rs:52-57` function `check_decay` | `src/config/validate.rs:63-68` function `check_unit_interval` | 89.04% | parallel per-section knob validators; each `check_*_knobs` range-checks a different fixed field set |
| `src/config/validate.rs:33-38` function `check_graph_seeds` | `src/config/validate.rs:44-49` function `check_top_k` | 88.94% | parallel per-section knob validators; each `check_*_knobs` range-checks a different fixed field set |
| `src/config/paths.rs:36-38` method `memories_dir` | `src/config/paths.rs:46-48` method `index_dir` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:75-77` method `auth_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:83-85` method `allowlist_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:89-91` method `sources_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:75-77` method `auth_file` | `src/config/paths.rs:83-85` method `allowlist_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:75-77` method `auth_file` | `src/config/paths.rs:89-91` method `sources_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:83-85` method `allowlist_file` | `src/config/paths.rs:89-91` method `sources_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/validate.rs:25-30` function `check_graph_hops` | `src/config/validate.rs:33-38` function `check_graph_seeds` | 88.07% | parallel per-section knob validators; each `check_*_knobs` range-checks a different fixed field set |
| `src/config/env.rs:115-132` method `apply_indexing_env` | `src/config/env.rs:211-225` method `apply_rank_env` | 87.83% | parallel per-section environment-override readers; each `apply_*_env` merges a different fixed set of knobs, symmetric by design |
| `src/config/env.rs:17-28` function `env_parse` | `src/config/env.rs:72-77` function `env_pair` | 87.53% | parallel per-section environment-override readers; each `apply_*_env` merges a different fixed set of knobs, symmetric by design |
| `src/config/env.rs:17-28` function `env_parse` | `src/config/env.rs:82-87` function `env_triple` | 86.91% | parallel per-section environment-override readers; each `apply_*_env` merges a different fixed set of knobs, symmetric by design |
| `src/config/validate.rs:16-21` function `check_rrf_k` | `src/config/validate.rs:44-49` function `check_top_k` | 86.84% | parallel per-section knob validators; each `check_*_knobs` range-checks a different fixed field set |
| `src/config/validate.rs:227-253` method `check_rank_knobs` | `src/config/validate.rs:256-290` method `check_prune_knobs` | 86.73% | parallel per-section knob validators; each `check_*_knobs` range-checks a different fixed field set |
| `src/config/env.rs:115-132` method `apply_indexing_env` | `src/config/env.rs:229-246` method `apply_prune_env` | 86.48% | parallel per-section environment-override readers; each `apply_*_env` merges a different fixed set of knobs, symmetric by design |
| `src/config/env.rs:173-184` method `apply_sync_env` | `src/config/env.rs:211-225` method `apply_rank_env` | 86.09% | parallel per-section environment-override readers; each `apply_*_env` merges a different fixed set of knobs, symmetric by design |
| `src/config/validate.rs:16-21` function `check_rrf_k` | `src/config/validate.rs:52-57` function `check_decay` | 85.80% | parallel per-section knob validators; each `check_*_knobs` range-checks a different fixed field set |
| `src/config/env.rs:187-192` method `apply_git_env` | `src/config/env.rs:249-254` method `apply_reinforce_env` | 85.66% | parallel per-section environment-override readers; each `apply_*_env` merges a different fixed set of knobs, symmetric by design |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:95-97` method `sources_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:75-77` method `auth_file` | `src/config/paths.rs:95-97` method `sources_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:75-77` method `auth_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:83-85` method `allowlist_file` | `src/config/paths.rs:95-97` method `sources_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:83-85` method `allowlist_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:89-91` method `sources_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/env.rs:173-184` method `apply_sync_env` | `src/config/env.rs:229-246` method `apply_prune_env` | 85.08% | parallel per-section environment-override readers; each `apply_*_env` merges a different fixed set of knobs, symmetric by design |

### `src/domains/code/ast/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/code/ast/extractor.rs:162-172` function `python_patterns` | `src/domains/code/ast/extractor.rs:174-185` function `go_patterns` | 91.82% | deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see AGENTS.md's `ast/` row |
| `src/domains/code/ast/extractor.rs:69-72` function `rust_compiled` | `src/domains/code/ast/extractor.rs:81-84` function `js_compiled` | 91.55% | deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see AGENTS.md's `ast/` row |
| `src/domains/code/ast/extractor.rs:69-72` function `rust_compiled` | `src/domains/code/ast/extractor.rs:87-90` function `python_compiled` | 91.55% | deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see AGENTS.md's `ast/` row |
| `src/domains/code/ast/extractor.rs:69-72` function `rust_compiled` | `src/domains/code/ast/extractor.rs:93-96` function `go_compiled` | 91.55% | deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see AGENTS.md's `ast/` row |
| `src/domains/code/ast/extractor.rs:81-84` function `js_compiled` | `src/domains/code/ast/extractor.rs:87-90` function `python_compiled` | 91.55% | deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see AGENTS.md's `ast/` row |
| `src/domains/code/ast/extractor.rs:81-84` function `js_compiled` | `src/domains/code/ast/extractor.rs:93-96` function `go_compiled` | 91.55% | deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see AGENTS.md's `ast/` row |
| `src/domains/code/ast/extractor.rs:87-90` function `python_compiled` | `src/domains/code/ast/extractor.rs:93-96` function `go_compiled` | 91.55% | deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see AGENTS.md's `ast/` row |
| `src/domains/code/ast/extractor.rs:69-72` function `rust_compiled` | `src/domains/code/ast/extractor.rs:75-78` function `ts_compiled` | 90.18% | deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see AGENTS.md's `ast/` row |
| `src/domains/code/ast/extractor.rs:75-78` function `ts_compiled` | `src/domains/code/ast/extractor.rs:81-84` function `js_compiled` | 90.18% | deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see AGENTS.md's `ast/` row |
| `src/domains/code/ast/extractor.rs:75-78` function `ts_compiled` | `src/domains/code/ast/extractor.rs:87-90` function `python_compiled` | 90.18% | deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see AGENTS.md's `ast/` row |
| `src/domains/code/ast/extractor.rs:75-78` function `ts_compiled` | `src/domains/code/ast/extractor.rs:93-96` function `go_compiled` | 90.18% | deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see AGENTS.md's `ast/` row |
| `src/domains/code/ast/pattern_cache.rs:31-40` function `cached` | `src/domains/code/ast/pattern_cache.rs:44-55` function `compile_patterns` | 85.31% | cache-hit vs. compile-miss halves of the same lookup |

### `src/domains/graph/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/graph/memory_rank.rs:78-89` function `push_direct_edges` | `src/domains/graph/memory_rank.rs:94-106` function `push_co_citation_edges` | 95.58% | direct-relation vs. co-citation edge pushes — twin PageRank edge loaders over twin queries |
| `src/domains/graph/imports.rs:337-354` function `python_imports` | `src/domains/graph/imports.rs:358-383` function `go_imports` | 92.32% | deliberate per-language import extraction, one arm per supported language |
| `src/domains/graph/imports.rs:321-325` function `ts_imports` | `src/domains/graph/imports.rs:328-332` function `js_imports` | 91.65% | deliberate per-language import extraction, one arm per supported language |
| `src/domains/graph/graph_nodes.rs:300-314` function `fetch_top_symbols` | `src/domains/graph/graph_nodes.rs:319-328` function `fetch_cited_by` | 88.98% | parallel graph-page builders / row-to-DTO mappers |
| `src/domains/graph/doc_link.rs:101-139` function `resolve_markdown_links` | `src/domains/graph/doc_link.rs:157-176` function `resolve_document_id` | 88.71% | parallel graph-page builders / row-to-DTO mappers |
| `src/domains/graph/imports.rs:55-70` function `extract_imports` | `src/domains/graph/imports.rs:223-248` function `rust_imports` | 87.31% | deliberate per-language import extraction, one arm per supported language |
| `src/domains/graph/imports.rs:223-248` function `rust_imports` | `src/domains/graph/imports.rs:358-383` function `go_imports` | 86.53% | deliberate per-language import extraction, one arm per supported language |
| `src/domains/graph/imports.rs:223-248` function `rust_imports` | `src/domains/graph/imports.rs:337-354` function `python_imports` | 86.42% | deliberate per-language import extraction, one arm per supported language |
| `src/domains/graph/query.rs:39-48` function `build_code_graph` | `src/domains/graph/query.rs:54-66` function `build_graph_page` | 85.67% | parallel graph-page builders / row-to-DTO mappers |
| `src/domains/graph/doc_link.rs:22-46` function `derive_after_document` | `src/domains/graph/doc_link.rs:101-139` function `resolve_markdown_links` | 85.28% | parallel graph-page builders / row-to-DTO mappers |
| `src/domains/graph/imports.rs:55-70` function `extract_imports` | `src/domains/graph/imports.rs:358-383` function `go_imports` | 85.12% | deliberate per-language import extraction, one arm per supported language |
| `src/domains/graph/imports.rs:55-70` function `extract_imports` | `src/domains/graph/imports.rs:337-354` function `python_imports` | 85.03% | deliberate per-language import extraction, one arm per supported language |

### `src/domains/sync/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
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

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/serve/routes.rs:239-249` function `guard_mutating` | `src/serve/routes.rs:256-261` function `guard_job` | 93.07% | parallel request-plumbing helpers over twin shapes |
| `src/serve/envelope.rs:68-76` method `unauthorized` | `src/serve/envelope.rs:98-106` method `read_only` | 90.50% | parallel response-envelope constructors, one per response shape |
| `src/serve/envelope.rs:68-76` method `unauthorized` | `src/serve/envelope.rs:115-123` method `confirmation_required` | 90.50% | parallel response-envelope constructors, one per response shape |
| `src/serve/router.rs:59-68` function `forbidden_response` | `src/serve/router.rs:72-77` function `unauthorized_response` | 89.56% | parallel request-plumbing helpers over twin shapes |
| `src/serve/envelope.rs:68-76` method `unauthorized` | `src/serve/envelope.rs:82-93` method `busy` | 89.25% | parallel response-envelope constructors, one per response shape |
| `src/serve/envelope.rs:98-106` method `read_only` | `src/serve/envelope.rs:115-123` method `confirmation_required` | 89.11% | parallel response-envelope constructors, one per response shape |
| `src/serve/envelope.rs:82-93` method `busy` | `src/serve/envelope.rs:98-106` method `read_only` | 87.47% | parallel response-envelope constructors, one per response shape |
| `src/serve/envelope.rs:82-93` method `busy` | `src/serve/envelope.rs:115-123` method `confirmation_required` | 87.47% | parallel response-envelope constructors, one per response shape |
| `src/serve/routes.rs:275-284` function `respond` | `src/serve/routes.rs:289-294` function `accepted` | 86.59% | parallel request-plumbing helpers over twin shapes |

### `src/serve/routes/memories/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
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

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/integrations/setup/apply.rs:86-96` function `git_hooks` | `src/domains/integrations/setup/apply.rs:99-116` function `index_code` | 89.66% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/plan.rs:63-75` function `data_dir` | `src/domains/integrations/setup/plan.rs:213-225` function `reinforce` | 87.43% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:61-81` function `agent_host` | `src/domains/integrations/setup/apply.rs:99-116` function `index_code` | 86.88% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:61-81` function `agent_host` | `src/domains/integrations/setup/apply.rs:86-96` function `git_hooks` | 86.88% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:54-58` function `data_dir` | `src/domains/integrations/setup/apply.rs:119-129` function `reinforce` | 86.78% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:34-50` function `one` | `src/domains/integrations/setup/apply.rs:99-116` function `index_code` | 86.55% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/detect.rs:207-219` function `hosts_present` | `src/domains/integrations/setup/detect.rs:226-232` function `hosts_installed` | 86.00% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:99-116` function `index_code` | `src/domains/integrations/setup/apply.rs:119-129` function `reinforce` | 85.27% | parallel per-integration setup arms, one per supported agent target |

### `src/domains/retrieval/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/retrieval/pipeline.rs:164-191` function `record_telemetry` | `src/domains/retrieval/pipeline.rs:226-237` function `record_query` | 92.98% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/score.rs:156-166` function `min_max_normalize` | `src/domains/retrieval/score.rs:183-189` function `max_normalize` | 91.94% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/fuse.rs:54-56` function `rrf_multi` | `src/domains/retrieval/fuse.rs:65-67` function `rrf_multi_weighted` | 90.21% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/scope.rs:70-75` method `window` | `src/domains/retrieval/scope.rs:126-128` method `window` | 89.43% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/suggest.rs:96-107` function `expansions` | `src/domains/retrieval/suggest.rs:116-127` function `recent` | 89.21% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/router.rs:254-271` function `route_vector_only` | `src/domains/retrieval/router.rs:340-358` function `route_lexical` | 88.14% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/fuse.rs:32-34` function `rrf` | `src/domains/retrieval/fuse.rs:40-42` function `rrf_k` | 86.10% | parallel scorers/normalizers over twin candidate shapes |

### `src/serve/jobs/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/serve/jobs/worker.rs:31-45` function `spawn_job` | `src/serve/jobs/worker.rs:73-94` function `spawn_job_for` | 92.05% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/registry.rs:320-327` function `active_in` | `src/serve/jobs/registry.rs:332-340` function `refuse_active` | 90.33% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/worker.rs:31-45` function `spawn_job` | `src/serve/jobs/worker.rs:53-64` function `spawn_job_with_id` | 88.65% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/registry.rs:287-289` method `subscribe` | `src/serve/jobs/registry.rs:293-298` method `subscribe_progress` | 88.20% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/registry.rs:210-212` method `subscribe_log` | `src/serve/jobs/registry.rs:287-289` method `subscribe` | 86.61% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/events.rs:30-37` method `new` | `src/serve/jobs/events.rs:57-64` method `new` | 85.35% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/events.rs:30-37` method `new` | `src/serve/jobs/events.rs:80-82` method `new` | 85.35% | parallel job-registry / worker arms, one per job kind |

### `src/domains/memories/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/memories/save.rs:358-367` function `near_duplicate` | `src/domains/memories/save.rs:372-391` function `near_duplicate_inner` | 88.69% | parallel memory command handlers over twin operations |
| `src/domains/memories/update.rs:282-297` function `mirror_row` | `src/domains/memories/update.rs:300-315` function `append_local_upsert` | 85.34% | parallel memory command handlers over twin operations |
| `src/domains/memories/list.rs:77-83` method `from` | `src/domains/memories/list.rs:115-128` method `from` | 85.16% | parallel memory command handlers over twin operations |

### `src/utilities/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/utilities/id_list.rs:30-40` function `parse_id_csv` | `src/utilities/id_list.rs:48-65` function `parse_symbol_id_csv` | 87.88% | parallel CSV-parse-into-Vec helpers for two id kinds |
| `src/utilities/fetch.rs:47-64` function `download` | `src/utilities/fetch.rs:67-93` function `final_url` | 86.19% | parallel fetch helpers over twin response shapes |
| `src/utilities/fetch.rs:95-124` function `exchange_curl` | `src/utilities/fetch.rs:126-171` function `exchange_wget` | 85.63% | parallel fetch helpers over twin response shapes |

### `src/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/serve.rs:145-151` method `swap_conn` | `src/serve.rs:162-171` method `reload_cfg` | 88.13% | trivial parallel getter methods on `AppState` |
| `src/serve.rs:194-196` method `repo` | `src/serve.rs:219-221` method `embed_cmd` | 87.31% | trivial parallel getter methods on `AppState` |

### `src/cli/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/cli/capture.rs:85-112` function `run_sources` | `src/cli/capture.rs:135-149` function `run_session` | 86.69% | parallel CLI command helpers over twin inputs |
| `src/cli/sync_render.rs:73-90` function `emit_daemon_status` | `src/cli/sync_render.rs:151-172` function `emit_verify` | 86.07% | parallel CLI command helpers over twin inputs |

### `src/cli/output/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/cli/output/prune.rs:36-43` function `write_list` | `src/cli/output/prune.rs:49-60` function `write_rows` | 93.08% | parallel render helpers, one per output shape |
| `src/cli/output/graph.rs:33-37` function `write_dot` | `src/cli/output/graph.rs:40-44` function `write_html` | 89.98% | parallel render helpers, one per output shape |

### `src/domains/sync/exchange/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/sync/exchange/import_state.rs:58-64` function `id_collision` | `src/domains/sync/exchange/import_state.rs:68-74` function `id_collision_for_test` | 95.78% | parallel import arms, one per exchanged record kind |
| `src/domains/sync/exchange/import_rules.rs:93-138` function `apply_restore` | `src/domains/sync/exchange/import_rules.rs:140-204` function `apply_upsert` | 86.56% | parallel import arms, one per exchanged record kind |

### `src/domains/sync/memory_store/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/sync/memory_store/git.rs:157-167` function `unmerged_paths` | `src/domains/sync/memory_store/git.rs:191-194` function `head` | 86.43% | parallel store-backend operations over twin transports |
| `src/domains/sync/memory_store/git.rs:191-194` function `head` | `src/domains/sync/memory_store/git.rs:203-211` function `push` | 85.99% | parallel store-backend operations over twin transports |

### `src/serve/routes/maint/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/serve/routes/maint/prune.rs:91-112` function `prune_apply` | `src/serve/routes/maint/prune.rs:151-166` function `gc` | 91.96% | parallel maintenance handlers — same extract-validate-respond skeleton per route |
| `src/serve/routes/maint/admin.rs:64-79` function `mine` | `src/serve/routes/maint/admin.rs:86-105` function `hooks_install` | 88.32% | parallel maintenance handlers — same extract-validate-respond skeleton per route |

### `src/domains/code/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/code/git_utils.rs:343-345` function `hook_installed` | `src/domains/code/git_utils.rs:361-364` function `hook_outdated` | 89.13% | parallel code-indexing helpers over twin inputs |

### `src/domains/documents/document/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/documents/document/extract.rs:70-84` function `extract_txt` | `src/domains/documents/document/extract.rs:86-95` function `extract_markdown` | 88.81% | parallel document extraction arms, one per source shape |

### `src/domains/learning/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/learning/learning_proposals.rs:181-189` function `parse_knobs` | `src/domains/learning/learning_proposals.rs:193-200` function `require_knobs` | 87.70% | parallel proposal builders over twin signal sources |

### `src/domains/maintenance/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/maintenance/reembed.rs:300-320` function `reembed_memories` | `src/domains/maintenance/reembed.rs:324-340` function `reembed_code` | 92.83% | parallel maintenance checks, one per checked surface |

### `src/domains/maintenance/doctor/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/maintenance/doctor/checks.rs:62-64` function `warn` | `src/domains/maintenance/doctor/checks.rs:67-69` function `fail` | 92.09% | parallel maintenance checks, one per checked surface |

### `src/domains/retrieval/unified/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/domains/retrieval/unified/fuse_domains.rs:213-226` function `code_hit` | `src/domains/retrieval/unified/fuse_domains.rs:229-248` function `doc_hit` | 89.78% | parallel per-domain fusion arms |

## Why none of this is urgent

Every group above falls into one of three shapes, none of which is a
correctness or maintainability emergency:

1. **Deliberate per-language / per-table-kind repetition**
   (`domains/code/ast/extractor.rs`, `domains/graph/imports.rs`,
   `store/vector.rs`, the `store/rebuild_copy_*` family) — the same operation
   repeated once per supported language or per parallel table, which is the
   documented shape of those modules (see AGENTS.md's `ast/`, `graph/` and
   `store/` rows). Generalizing these into one parameterized function would
   trade readable, greppable per-case functions for an indirection layer, for
   five-or-fewer call sites each.
2. **Parallel accessor / CRUD pairs** (`config/paths.rs` path joins,
   `store/sources.rs` source-vs-file rows, `store/documents.rs` fetch helpers,
   `store/feedback.rs` used-vs-irrelevant counters, and the `serve/routes/`
   handler families) — twin small functions over twin data shapes.
   `similarity-rs`'s APTED tree-edit-distance scoring is naturally high on
   these because they *are* structurally identical by design; the alternative
   is a generic helper taking a table name or route shape as a parameter, which
   most of these modules deliberately avoid to keep each function's SQL and
   validation inline and auditable.
3. **The canonical "parallel `Config`-section `apply()` methods" case**
   (`config/file.rs`, `config/env.rs`, `config/validate.rs`). Each
   `{Indexing,Rank,Prune}Config::apply` / `apply_*_env` / `check_*_knobs`
   method merges or validates a different fixed set of fields; the shape is
   identical because the sections are deliberately symmetric, not because of
   copy-paste that drifted.

None of the 242 pairs crosses a genuine behavioral seam — no pair spans two
independently-evolving business rules that merely look alike today. Follow-up
dedup work, if undertaken, should extract shared helpers pair-by-pair and shrink
both this table and `dup-baseline.txt` accordingly, tracked as debt rather than
as blocking work for any single PR.
