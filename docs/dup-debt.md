# Duplication debt baseline (`scripts/dup-check.sh`)

Status: documented baseline, tracked by a count ratchet · Owner: whoever burns
a pair down next

`scripts/dup-check.sh` was silently non-functional before Phase 6 of the
toolu-conventions migration (`docs/toolu/specs/folder-structure-migration.md`):
it invoked `similarity-rs` with CLI flags the installed version doesn't accept
and grepped for output shapes the tool never produces, so it always fell
through to a green no-op. Phase 6 fixed the invocation (correct flags,
`--fail-on-duplicates` as the source of truth). Run for real for the first
time, it finds **113 near-duplicate function/method pairs at threshold
0.85** across `src/` and `scripts/`. Every one of them predates the
migration — none was introduced by folder moves, `mod.rs` -> `<dir>.rs`
flattening, or the colocated-tests rewrite; they are pre-existing shape
duplication that a broken gate simply never caught.

This document is the baseline snapshot **at fix time**, mirroring the
`docs/lint-debt.md`-style pattern used for the D7 `clippy::pedantic`
debt declared in `Cargo.toml`: numbers are measured, not estimated, and the
scope of this task is to record and grandfather them, not to fix them. Do not
add to this list without the same treatment (measure, document, ratchet).

## How the gate treats this

`similarity-rs` has no native per-pair suppression flag (confirmed via
`similarity-rs --help` — there is `--exclude` for directories and
`--filter-function` / `--filter-function-body` for substring filters, nothing
addressing an individual reported pair). A per-pair allowlist would therefore
mean reimplementing pair identity matching by hand, which is worse than the
problem it solves. `scripts/dup-check.sh` instead runs a **count ratchet**:

- The known baseline count (`113`) lives in `dup-baseline.txt`,
  a single tracked integer.
- Every run of `scripts/dup-check.sh` re-scans `src/` + `scripts/` and compares
  the current duplicate-pair count against that baseline.
- **Current count > baseline → gate fails.** New duplication introduced by a
  future PR is caught, exactly as if the gate had always worked.
- **Current count <= baseline → gate passes.** The 113 pre-existing pairs below
  are grandfathered, not hidden: they are enumerated in full in the table
  below, with the reason each is considered non-urgent debt rather than an
  active defect.
- Burning a pair down (actually deduplicating two functions) is expected to
  lower the count. Whoever does that work also lowers the integer in
  `dup-baseline.txt` and deletes the corresponding row(s)
  below in the same PR — the ratchet only ever tightens.

Re-run the raw scan yourself with:

```bash
similarity-rs --threshold 0.85 --fail-on-duplicates $(git ls-files 'src/*.rs' 'scripts/*.sh' | grep -v '/tests/')
```

## The 113 pairs, grouped by area

Grouped by the top-level `src/<area>/` directory of the first function in
each pair (a handful of pairs live directly under `src/` — `cli.rs`,
`serve.rs`). Similarity is `similarity-rs`'s APTED-based score at
`--threshold 0.85`; line ranges are the function/method body span at scan
time and will drift with unrelated edits to the same file (harmless — the
ratchet re-scans fresh each run, it does not pin line numbers).

### `src/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/cli.rs:284-294` function `parse_id_csv` | `src/cli.rs:302-319` function `parse_symbol_id_csv` | 87.88% | parallel CSV-parse-into-Vec helpers for two id kinds (plain id vs. symbol id) |
| `src/serve.rs:94-96` method `repo` | `src/serve.rs:104-106` method `embed_cmd` | 87.31% | trivial parallel getter methods on `AppState` |

### `src/ast/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/ast/pattern_cache.rs:31-40` function `cached` | `src/ast/pattern_cache.rs:44-55` function `compile_patterns` | 85.31% | cache-hit vs. compile-miss halves of the same lookup |
| `src/ast/extractor.rs:162-172` function `python_patterns` | `src/ast/extractor.rs:174-185` function `go_patterns` | 91.82% | the deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see CLAUDE.md `ast/` row |
| `src/ast/extractor.rs:69-72` function `rust_compiled` | `src/ast/extractor.rs:81-84` function `js_compiled` | 91.55% | the deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see CLAUDE.md `ast/` row |
| `src/ast/extractor.rs:69-72` function `rust_compiled` | `src/ast/extractor.rs:87-90` function `python_compiled` | 91.55% | the deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see CLAUDE.md `ast/` row |
| `src/ast/extractor.rs:69-72` function `rust_compiled` | `src/ast/extractor.rs:93-96` function `go_compiled` | 91.55% | the deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see CLAUDE.md `ast/` row |
| `src/ast/extractor.rs:81-84` function `js_compiled` | `src/ast/extractor.rs:87-90` function `python_compiled` | 91.55% | the deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see CLAUDE.md `ast/` row |
| `src/ast/extractor.rs:81-84` function `js_compiled` | `src/ast/extractor.rs:93-96` function `go_compiled` | 91.55% | the deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see CLAUDE.md `ast/` row |
| `src/ast/extractor.rs:87-90` function `python_compiled` | `src/ast/extractor.rs:93-96` function `go_compiled` | 91.55% | the deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see CLAUDE.md `ast/` row |
| `src/ast/extractor.rs:69-72` function `rust_compiled` | `src/ast/extractor.rs:75-78` function `ts_compiled` | 90.18% | the deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see CLAUDE.md `ast/` row |
| `src/ast/extractor.rs:75-78` function `ts_compiled` | `src/ast/extractor.rs:81-84` function `js_compiled` | 90.18% | the deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see CLAUDE.md `ast/` row |
| `src/ast/extractor.rs:75-78` function `ts_compiled` | `src/ast/extractor.rs:87-90` function `python_compiled` | 90.18% | the deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see CLAUDE.md `ast/` row |
| `src/ast/extractor.rs:75-78` function `ts_compiled` | `src/ast/extractor.rs:93-96` function `go_compiled` | 90.18% | the deliberate per-language repetition for the five supported languages (rust/typescript/javascript/python/go) — same shape per language, see CLAUDE.md `ast/` row |

### `src/cli/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/cli/graph.rs:148-157` function `build_code_graph` | `src/cli/graph.rs:163-175` function `build_graph_page` | 85.67% | parallel graph-page builders / SQL-row-to-DTO mappers |
| `src/cli/graph.rs:195-202` function `map_edge` | `src/cli/graph.rs:352-363` function `map_node_row` | 89.09% | parallel graph-page builders / SQL-row-to-DTO mappers |
| `src/cli/lazy_reindex.rs:173-187` function `read_repo_marker` | `src/cli/lazy_reindex.rs:203-214` function `read_last_trigger` | 88.77% | parallel stored-marker readers (repo marker vs. last-trigger timestamp) |
| `src/cli/rebuild.rs:246-276` function `copy_code_index_tables` | `src/cli/rebuild.rs:435-473` function `copy_event_and_mined_tables` | 85.45% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:315-339` function `copy_code_markers` | `src/cli/rebuild.rs:435-473` function `copy_event_and_mined_tables` | 85.95% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:379-398` function `copy_feedback_tables` | `src/cli/rebuild.rs:435-473` function `copy_event_and_mined_tables` | 89.12% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:246-276` function `copy_code_index_tables` | `src/cli/rebuild.rs:315-339` function `copy_code_markers` | 89.58% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:246-276` function `copy_code_index_tables` | `src/cli/rebuild.rs:405-427` function `copy_retrieval_log` | 85.64% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:246-276` function `copy_code_index_tables` | `src/cli/rebuild.rs:379-398` function `copy_feedback_tables` | 89.45% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:315-339` function `copy_code_markers` | `src/cli/rebuild.rs:405-427` function `copy_retrieval_log` | 87.29% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:315-339` function `copy_code_markers` | `src/cli/rebuild.rs:379-398` function `copy_feedback_tables` | 85.67% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:288-303` function `copy_mined_edges` | `src/cli/rebuild.rs:315-339` function `copy_code_markers` | 90.17% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:379-398` function `copy_feedback_tables` | `src/cli/rebuild.rs:405-427` function `copy_retrieval_log` | 85.35% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:315-339` function `copy_code_markers` | `src/cli/rebuild.rs:345-359` function `copy_code_virtual_tables` | 89.46% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:288-303` function `copy_mined_edges` | `src/cli/rebuild.rs:405-427` function `copy_retrieval_log` | 88.51% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:288-303` function `copy_mined_edges` | `src/cli/rebuild.rs:379-398` function `copy_feedback_tables` | 85.20% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:345-359` function `copy_code_virtual_tables` | `src/cli/rebuild.rs:379-398` function `copy_feedback_tables` | 86.46% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:288-303` function `copy_mined_edges` | `src/cli/rebuild.rs:345-359` function `copy_code_virtual_tables` | 86.08% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:235-243` function `copy_code_tables_inner` | `src/cli/rebuild.rs:345-359` function `copy_code_virtual_tables` | 85.43% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:477-484` function `old_table_exists` | `src/cli/rebuild.rs:490-497` function `old_column_exists` | 85.93% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/rebuild.rs:235-243` function `copy_code_tables_inner` | `src/cli/rebuild.rs:369-373` function `copy_learning_tables_inner` | 92.00% | one `copy_*_tables` helper per table group in the rebuild copy step, same shape by design (deliberately enumerated per group rather than generalized) |
| `src/cli/index_code.rs:229-278` function `write_symbol` | `src/cli/index_code.rs:290-330` function `emit_symbol_jsonl` | 88.86% | parallel symbol-to-output-row helpers for TTY vs. JSONL emission |
| `src/cli/save.rs:261-275` function `near_duplicate` | `src/cli/save.rs:280-294` function `near_duplicate_inner` | 89.37% | outer-wrapper / inner-impl split of the same near-duplicate SimHash check |

### `src/config/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/config/env.rs:103-120` method `apply_indexing_env` | `src/config/env.rs:210-227` method `apply_prune_env` | 86.48% | parallel per-section `apply_*_env` methods (one per Config sub-struct) plus arity-variant env parsers (`env_parse`/`env_pair`/`env_triple`), same shape by design |
| `src/config/env.rs:192-206` method `apply_rank_env` | `src/config/env.rs:210-227` method `apply_prune_env` | 92.98% | parallel per-section `apply_*_env` methods (one per Config sub-struct) plus arity-variant env parsers (`env_parse`/`env_pair`/`env_triple`), same shape by design |
| `src/config/env.rs:103-120` method `apply_indexing_env` | `src/config/env.rs:160-173` method `apply_git_env` | 91.18% | parallel per-section `apply_*_env` methods (one per Config sub-struct) plus arity-variant env parsers (`env_parse`/`env_pair`/`env_triple`), same shape by design |
| `src/config/env.rs:103-120` method `apply_indexing_env` | `src/config/env.rs:192-206` method `apply_rank_env` | 87.83% | parallel per-section `apply_*_env` methods (one per Config sub-struct) plus arity-variant env parsers (`env_parse`/`env_pair`/`env_triple`), same shape by design |
| `src/config/env.rs:160-173` method `apply_git_env` | `src/config/env.rs:192-206` method `apply_rank_env` | 85.03% | parallel per-section `apply_*_env` methods (one per Config sub-struct) plus arity-variant env parsers (`env_parse`/`env_pair`/`env_triple`), same shape by design |
| `src/config/env.rs:17-28` function `env_parse` | `src/config/env.rs:61-66` function `env_pair` | 87.53% | parallel per-section `apply_*_env` methods (one per Config sub-struct) plus arity-variant env parsers (`env_parse`/`env_pair`/`env_triple`), same shape by design |
| `src/config/env.rs:17-28` function `env_parse` | `src/config/env.rs:71-76` function `env_triple` | 86.91% | parallel per-section `apply_*_env` methods (one per Config sub-struct) plus arity-variant env parsers (`env_parse`/`env_pair`/`env_triple`), same shape by design |
| `src/config/env.rs:61-66` function `env_pair` | `src/config/env.rs:71-76` function `env_triple` | 92.22% | parallel per-section `apply_*_env` methods (one per Config sub-struct) plus arity-variant env parsers (`env_parse`/`env_pair`/`env_triple`), same shape by design |
| `src/config/paths.rs:75-77` method `sources_file` | `src/config/paths.rs:81-83` method `sources_lock_file` | 93.78% | parallel single-segment path-join accessors on `Paths` |
| `src/config/paths.rs:36-38` method `memories_dir` | `src/config/paths.rs:46-48` method `index_dir` | 88.78% | parallel single-segment path-join accessors on `Paths` |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:75-77` method `sources_file` | 88.78% | parallel single-segment path-join accessors on `Paths` |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:81-83` method `sources_lock_file` | 85.66% | parallel single-segment path-join accessors on `Paths` |
| `src/config/file.rs:167-180` method `apply` | `src/config/file.rs:268-287` method `apply` | 91.32% | parallel per-section `apply()` methods (`IndexingConfig`/`RankConfig`/`PruneConfig`) — the canonical case cited in this doc's intro, same TOML-merge shape by design |
| `src/config/file.rs:219-232` method `apply` | `src/config/file.rs:268-287` method `apply` | 91.32% | parallel per-section `apply()` methods (`IndexingConfig`/`RankConfig`/`PruneConfig`) — the canonical case cited in this doc's intro, same TOML-merge shape by design |
| `src/config/file.rs:167-180` method `apply` | `src/config/file.rs:219-232` method `apply` | 92.58% | parallel per-section `apply()` methods (`IndexingConfig`/`RankConfig`/`PruneConfig`) — the canonical case cited in this doc's intro, same TOML-merge shape by design |
| `src/config/validate.rs:208-234` method `check_rank_knobs` | `src/config/validate.rs:237-265` method `check_prune_knobs` | 87.68% | parallel per-knob-group range-validation helpers, same shape by design |
| `src/config/validate.rs:197-205` method `check_indexing_knobs` | `src/config/validate.rs:296-304` method `check_reinforce_knobs` | 89.70% | parallel per-knob-group range-validation helpers, same shape by design |
| `src/config/validate.rs:40-45` function `check_decay` | `src/config/validate.rs:51-56` function `check_unit_interval` | 89.04% | parallel per-knob-group range-validation helpers, same shape by design |
| `src/config/validate.rs:24-29` function `check_graph_hops` | `src/config/validate.rs:32-37` function `check_graph_seeds` | 88.07% | parallel per-knob-group range-validation helpers, same shape by design |
| `src/config/validate.rs:15-20` function `check_rrf_k` | `src/config/validate.rs:40-45` function `check_decay` | 85.80% | parallel per-knob-group range-validation helpers, same shape by design |

### `src/document/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/document/extract.rs:70-84` function `extract_txt` | `src/document/extract.rs:86-95` function `extract_markdown` | 88.81% | parallel read-and-chunk extractors for two plain-text document kinds |

### `src/graph/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/graph/doc_link.rs:111-149` function `resolve_markdown_links` | `src/graph/doc_link.rs:174-199` function `resolve_document_id` | 85.67% | parallel document/memory-reference resolvers, same link-walk shape by design |
| `src/graph/doc_link.rs:21-45` function `derive_after_document` | `src/graph/doc_link.rs:111-149` function `resolve_markdown_links` | 85.28% | parallel document/memory-reference resolvers, same link-walk shape by design |
| `src/graph/doc_link.rs:78-105` function `resolve_memory_references` | `src/graph/doc_link.rs:174-199` function `resolve_document_id` | 85.26% | parallel document/memory-reference resolvers, same link-walk shape by design |
| `src/graph/edges.rs:116-125` function `insert_weighted` | `src/graph/edges.rs:131-141` function `current_weight` | 86.15% | parallel weighted-upsert / delete-by-predicate helpers on the same `edges` table |
| `src/graph/edges.rs:97-106` function `insert_at` | `src/graph/edges.rs:116-125` function `insert_weighted` | 85.55% | parallel weighted-upsert / delete-by-predicate helpers on the same `edges` table |
| `src/graph/edges.rs:197-203` function `delete_outgoing` | `src/graph/edges.rs:207-213` function `delete_touching` | 93.65% | parallel weighted-upsert / delete-by-predicate helpers on the same `edges` table |
| `src/graph/memory_rank.rs:116-127` function `push_direct_edges` | `src/graph/memory_rank.rs:132-144` function `push_co_citation_edges` | 95.57% | parallel edge-push helpers for two edge kinds (direct vs. co-citation) into the same rank graph |
| `src/graph/imports.rs:223-248` function `rust_imports` | `src/graph/imports.rs:358-383` function `go_imports` | 86.53% | the deliberate per-language import-resolution repetition (rust/python/go/ts/js) — same shape per language, see CLAUDE.md `graph/` row |
| `src/graph/imports.rs:337-354` function `python_imports` | `src/graph/imports.rs:358-383` function `go_imports` | 92.32% | the deliberate per-language import-resolution repetition (rust/python/go/ts/js) — same shape per language, see CLAUDE.md `graph/` row |
| `src/graph/imports.rs:223-248` function `rust_imports` | `src/graph/imports.rs:337-354` function `python_imports` | 86.42% | the deliberate per-language import-resolution repetition (rust/python/go/ts/js) — same shape per language, see CLAUDE.md `graph/` row |
| `src/graph/imports.rs:55-70` function `extract_imports` | `src/graph/imports.rs:223-248` function `rust_imports` | 87.31% | the deliberate per-language import-resolution repetition (rust/python/go/ts/js) — same shape per language, see CLAUDE.md `graph/` row |
| `src/graph/imports.rs:55-70` function `extract_imports` | `src/graph/imports.rs:358-383` function `go_imports` | 85.12% | the deliberate per-language import-resolution repetition (rust/python/go/ts/js) — same shape per language, see CLAUDE.md `graph/` row |
| `src/graph/imports.rs:55-70` function `extract_imports` | `src/graph/imports.rs:337-354` function `python_imports` | 85.03% | the deliberate per-language import-resolution repetition (rust/python/go/ts/js) — same shape per language, see CLAUDE.md `graph/` row |
| `src/graph/imports.rs:321-325` function `ts_imports` | `src/graph/imports.rs:328-332` function `js_imports` | 91.65% | the deliberate per-language import-resolution repetition (rust/python/go/ts/js) — same shape per language, see CLAUDE.md `graph/` row |

### `src/output/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/output/graph.rs:114-118` function `write_dot` | `src/output/graph.rs:121-125` function `write_html` | 89.98% | parallel DOT vs. HTML emitters over the same graph walk |

### `src/retrieval/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/retrieval/fuse.rs:54-56` function `rrf_multi` | `src/retrieval/fuse.rs:65-67` function `rrf_multi_weighted` | 90.21% | weighted vs. unweighted variants of the same RRF fusion formula |
| `src/retrieval/fuse.rs:32-34` function `rrf` | `src/retrieval/fuse.rs:40-42` function `rrf_k` | 86.10% | weighted vs. unweighted variants of the same RRF fusion formula |
| `src/retrieval/router.rs:228-243` function `route_vector_only` | `src/retrieval/router.rs:314-330` function `route_lexical` | 88.51% | single-leg routing variants of the shared two-leg router, same scaffolding |
| `src/retrieval/scope.rs:59-64` method `window` | `src/retrieval/scope.rs:115-117` method `window` | 89.43% | two different structs' same-named `window` accessor, same shape |
| `src/retrieval/score.rs:155-165` function `min_max_normalize` | `src/retrieval/score.rs:182-188` function `max_normalize` | 91.94% | min-max vs. max-only variants of the same normalization formula |
| `src/retrieval/pipeline.rs:186-213` function `record_telemetry` | `src/retrieval/pipeline.rs:248-259` function `record_query` | 92.69% | parallel telemetry/query log-row inserts, same shape |

### `src/stats/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/stats/code_feedback.rs:106-115` function `upsert_code_used` | `src/stats/code_feedback.rs:121-130` function `upsert_code_irrelevant` | 88.18% | parallel counter-upsert helpers for two feedback signals (used vs. irrelevant) |
| `src/stats/feedback.rs:106-119` function `insert_event` | `src/stats/feedback.rs:132-146` function `record_implicit_used` | 85.25% | parallel event-insert / counter-upsert helpers for two feedback signals |
| `src/stats/feedback.rs:77-85` function `upsert_used` | `src/stats/feedback.rs:91-99` function `upsert_irrelevant` | 88.38% | parallel event-insert / counter-upsert helpers for two feedback signals |

### `src/store/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/store/memory_row.rs:136-151` function `relation_edge_stamps` | `src/store/memory_row.rs:226-260` function `insert_relation_edges` | 85.18% | edge-row construction helpers sharing the same relation-materialization shape |
| `src/store/memory_meta.rs:95-115` function `attach_tags` | `src/store/memory_meta.rs:121-156` function `attach_references` | 89.52% | parallel batched attach-by-id helpers for two metadata columns (tags vs. references) |
| `src/store/sources.rs:59-72` function `upsert` | `src/store/sources.rs:203-221` function `upsert_file` | 90.50% | parallel source-row vs. file-row CRUD helpers on twin tables |
| `src/store/sources.rs:112-122` function `row_from_sql` | `src/store/sources.rs:309-323` function `file_row_from_sql` | 93.26% | parallel source-row vs. file-row CRUD helpers on twin tables |
| `src/store/sources.rs:95-104` function `get` | `src/store/sources.rs:253-257` function `get_file` | 92.10% | parallel source-row vs. file-row CRUD helpers on twin tables |
| `src/store/sources.rs:76-79` function `delete` | `src/store/sources.rs:95-104` function `get` | 85.93% | parallel source-row vs. file-row CRUD helpers on twin tables |
| `src/store/vector.rs:96-131` function `knn_memory` | `src/store/vector.rs:161-189` function `knn_code` | 85.52% | memory vs. code are two parallel vec0 tables by design (see CLAUDE.md); knn/dim/insert helpers mirror each other one-for-one |
| `src/store/vector.rs:28-36` function `dim_memory` | `src/store/vector.rs:39-47` function `dim_code` | 94.90% | memory vs. code are two parallel vec0 tables by design (see CLAUDE.md); knn/dim/insert helpers mirror each other one-for-one |
| `src/store/vector.rs:50-58` function `insert_memory` | `src/store/vector.rs:142-150` function `insert_code` | 90.23% | memory vs. code are two parallel vec0 tables by design (see CLAUDE.md); knn/dim/insert helpers mirror each other one-for-one |
| `src/store/code_row.rs:126-142` function `purge_file_symbols` | `src/store/code_row.rs:167-174` function `upsert_repo_root` | 85.80% | parallel repo-format stamping / row-upsert helpers, same SQL shape |
| `src/store/code_row.rs:58-70` function `ensure_repo_format` | `src/store/code_row.rs:77-84` function `stamp_repo_format` | 87.73% | parallel repo-format stamping / row-upsert helpers, same SQL shape |
| `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | `src/store/fts_memory.rs:135-184` function `run_memory_match` | 86.74% | the deliberate 4-tier lexical fallback ladder (strict/relaxed/subtoken/expanded) behind the `run_memory_match` choke point — same query shape, different tier predicate, by design |
| `src/store/fts_memory.rs:40-58` function `search_memory` | `src/store/fts_memory.rs:82-100` function `search_memory_subtokens` | 98.78% | the deliberate 4-tier lexical fallback ladder (strict/relaxed/subtoken/expanded) behind the `run_memory_match` choke point — same query shape, different tier predicate, by design |
| `src/store/fts_memory.rs:40-58` function `search_memory` | `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | 89.24% | the deliberate 4-tier lexical fallback ladder (strict/relaxed/subtoken/expanded) behind the `run_memory_match` choke point — same query shape, different tier predicate, by design |
| `src/store/fts_memory.rs:82-100` function `search_memory_subtokens` | `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | 88.50% | the deliberate 4-tier lexical fallback ladder (strict/relaxed/subtoken/expanded) behind the `run_memory_match` choke point — same query shape, different tier predicate, by design |
| `src/store/fts_memory.rs:40-58` function `search_memory` | `src/store/fts_memory.rs:63-73` function `search_memory_relaxed` | 98.09% | the deliberate 4-tier lexical fallback ladder (strict/relaxed/subtoken/expanded) behind the `run_memory_match` choke point — same query shape, different tier predicate, by design |
| `src/store/fts_memory.rs:63-73` function `search_memory_relaxed` | `src/store/fts_memory.rs:82-100` function `search_memory_subtokens` | 97.35% | the deliberate 4-tier lexical fallback ladder (strict/relaxed/subtoken/expanded) behind the `run_memory_match` choke point — same query shape, different tier predicate, by design |
| `src/store/fts_memory.rs:63-73` function `search_memory_relaxed` | `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | 89.02% | the deliberate 4-tier lexical fallback ladder (strict/relaxed/subtoken/expanded) behind the `run_memory_match` choke point — same query shape, different tier predicate, by design |
| `src/store/documents.rs:159-178` function `get_chunk` | `src/store/documents.rs:185-195` function `get_document_path` | 88.48% | parallel single-row-by-id fetch/delete helpers on the documents table |
| `src/store/documents.rs:73-82` function `get_document` | `src/store/documents.rs:185-195` function `get_document_path` | 92.10% | parallel single-row-by-id fetch/delete helpers on the documents table |
| `src/store/documents.rs:90-93` function `delete_document` | `src/store/documents.rs:185-195` function `get_document_path` | 86.58% | parallel single-row-by-id fetch/delete helpers on the documents table |
| `src/store/documents.rs:73-82` function `get_document` | `src/store/documents.rs:90-93` function `delete_document` | 86.53% | parallel single-row-by-id fetch/delete helpers on the documents table |
| `src/store/code_ref.rs:39-52` function `materialize` | `src/store/code_ref.rs:78-92` function `upsert` | 89.56% | parallel edge-row upsert/materialize helpers, same shape |

### `src/tui/`

| Pair A | Pair B | Similarity | Why it's debt, not urgent |
| --- | --- | --- | --- |
| `src/tui/preview.rs:20-28` function `memory_preview` | `src/tui/preview.rs:31-39` function `code_preview` | 91.13% | parallel truncate-and-format preview helpers for memory vs. code hits |
| `src/tui/preview.rs:12-17` function `preview_text` | `src/tui/preview.rs:31-39` function `code_preview` | 87.88% | parallel truncate-and-format preview helpers for memory vs. code hits |
| `src/tui/preview.rs:12-17` function `preview_text` | `src/tui/preview.rs:20-28` function `memory_preview` | 87.02% | parallel truncate-and-format preview helpers for memory vs. code hits |
| `src/tui/app.rs:178-203` method `apply` | `src/tui/app.rs:253-269` method `semantic` | 86.48% | parallel state-mutation methods on `App`, one pair per feature (paging, hit-setters, semantic/copy actions) |
| `src/tui/app.rs:253-269` method `semantic` | `src/tui/app.rs:272-278` method `copy_id` | 88.25% | parallel state-mutation methods on `App`, one pair per feature (paging, hit-setters, semantic/copy actions) |
| `src/tui/app.rs:221-229` method `page_next` | `src/tui/app.rs:232-240` method `page_prev` | 91.04% | parallel state-mutation methods on `App`, one pair per feature (paging, hit-setters, semantic/copy actions) |
| `src/tui/app.rs:164-168` method `set_memory_hits` | `src/tui/app.rs:171-175` method `set_code_hits` | 89.68% | parallel state-mutation methods on `App`, one pair per feature (paging, hit-setters, semantic/copy actions) |
| `src/tui/view/layout.rs:31-36` function `render_search` | `src/tui/view/layout.rs:39-49` function `render_status` | 88.18% | parallel ratatui layout-chunk-and-draw helpers for two panes |


## Why none of this is urgent

Every group above falls into one of three shapes, none of which is a
correctness or maintainability emergency:

1. **Deliberate per-language / per-table-kind repetition** (`ast/extractor.rs`,
   `graph/imports.rs`, `store/vector.rs`, `cli/rebuild.rs`'s `copy_*_tables`
   family) — the same operation repeated once per supported language or
   per parallel table, which is the documented shape of those modules (see
   `CLAUDE.md`'s `ast/`, `graph/`, and `store/` rows). Generalizing these
   into one parameterized function would trade readable, greppable
   per-case functions for an indirection layer, for five-or-fewer call
   sites each.
2. **Parallel accessor / CRUD pairs** (`config/paths.rs` path joins,
   `store/sources.rs` source-vs-file rows, `store/documents.rs` fetch
   helpers, `stats/*.rs` used-vs-irrelevant counters) — twin small
   functions over twin data shapes. Similarity-rs's APTED tree-edit-distance
   scoring is naturally high on these because they *are* structurally
   identical by design; the alternative is a generic helper taking a
   table-name parameter, which most of these modules deliberately avoid to
   keep each function's SQL inline and auditable.
3. **The canonical "parallel `Config`-section `apply()` methods" case**
   (`config/file.rs`, `config/env.rs`, `config/validate.rs`) — cited in this
   task's own framing. Each `{Indexing,Rank,Prune}Config::apply` /
   `apply_*_env` / `check_*_knobs` method merges or validates a different
   fixed set of fields; the shape is identical because the sections are
   deliberately symmetric, not because of copy-paste that drifted.

None of the 113 pairs cross a genuine behavioral seam (e.g., no pair spans
two independently-evolving business rules that happen to look alike today).
Follow-up dedup work, if undertaken, should extract shared helpers
pair-by-pair and shrink both this table and `dup-baseline.txt` accordingly —
tracked as debt, not as blocking work for any single PR.
