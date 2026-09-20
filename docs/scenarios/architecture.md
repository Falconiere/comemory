# `comemory architecture`

Scaffold a component-level architecture model from the indexed code graph,
store it as one memory tagged `architecture`, draw it, and check it for drift.
Nested: `scaffold` / `save` / `show` / `check` / `learn`. The model is the
console's drawing input; `--format mermaid` is the terminal's. `learn` runs
the agent command the caller passes — comemory ships none and detects none.

**Runnable tests:** `tests/cli__architecture.rs`, `tests/cli__architecture_2.rs`,
`tests/cli__architecture_3.rs`, colocated `src/domains/architecture/tests/*`

**HTTP:** none — the console reads the model through
`GET /api/v1/memories?tag=architecture` (`transport: "cli-only"`).

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`save [FILE]` — model file, default `-` (stdin). No other subcommand takes one.

## Flags

| Flag | Subcommand | Default | Effect |
| --- | --- | --- | --- |
| `--repo` | all | cwd's git repo | Repo label to work on |
| `--depth` | `scaffold`, `check`, `learn` | `2` | Directory-prefix depth a component clusters at (>= 1) |
| `--max-components` | `scaffold`, `check`, `learn` | `120` | Keep this many components, highest rank first (>= 1) |
| `--min-edge-weight` | `scaffold`, `check`, `learn` | `1` | Drop component edges below this weight (>= 1) |
| `--format` | `show` | `json` | `json` \| `mermaid`; global `--json` overrides it |
| `--command` | `learn` | required | Agent template; must contain `{prompt_file}` or `{prompt}` |
| `--timeout` | `learn` | `600` | Kill the agent command after this many seconds |
| `--dry-run` | `learn` | false | Write the prompt and stop; spawn nothing |

## Scenarios

### architecture-01 Scaffold clusters the index

- **Flags:** `--repo`, `--json`
- **Setup:** two-directory git repo whose first directory imports the second,
  indexed by the real `index-code`
- **Command:** `comemory architecture scaffold --repo r --json`
- **Expect:** `schema: 1`, `source: scaffold`, components `src_a` / `src_b`
  whose members are `src/a` / `src/b`, and one `imports` edge `src_a → src_b`.
  A second run is byte-identical apart from `generated_at`.
- **Covered by:** `tests/cli__architecture.rs::scaffold_clusters_indexed_directories_and_projects_mined_edges`,
  `tests/cli__architecture.rs::scaffold_is_byte_stable_across_runs`

### architecture-02 Depth and the other shape knobs

- **Flags:** `--depth`, `--max-components`, `--min-edge-weight`
- **Command:** `comemory architecture scaffold --repo r --depth 1 --json`
- **Expect:** both directories collapse into one `src` component with two
  files; intra-component edges are dropped, so `edges` is empty.
  `--max-components` truncates in rank order and `--min-edge-weight` drops
  weaker edges (same `scaffold::Options` the check path uses).
  Each knob rejects `0` as a clap usage error (exit 2). Without `--json`,
  `scaffold` prints the component table instead of JSON.
- **Covered by:** `tests/cli__architecture.rs::depth_one_collapses_the_directories_into_one_component`,
  `tests/cli__architecture.rs::scaffold_without_json_prints_the_component_table`,
  `tests/cli__architecture_2.rs::a_zero_knob_is_a_usage_error_rather_than_an_empty_model`,
  `src/domains/architecture/tests/cluster.rs`

### architecture-03 Save validates against the index

- **Flags:** `--repo`, positional `FILE`
- **Command:** `comemory architecture save model.json --repo r`
- **Expect:** one live memory, `kind: note`, `tag: architecture`; a member
  matching no indexed file exits 64 naming it and writes nothing; duplicate
  ids, unknown groups, dangling edge endpoints, malformed ids, a foreign
  `repo` and an oversized model all exit 64; an unknown field exits 65.
- **Covered by:** `tests/cli__architecture.rs::a_saved_model_becomes_the_repos_single_tagged_memory`,
  `tests/cli__architecture_2.rs::a_member_that_matches_no_indexed_file_is_refused_and_nothing_is_written`,
  `tests/cli__architecture_2.rs::every_structural_violation_is_refused_by_name`,
  `tests/cli__architecture_2.rs::an_unknown_field_is_a_data_error_and_an_oversized_model_is_refused`

### architecture-04 Supersede chain

- **Flags:** `--repo`, `--json`
- **Command:** `comemory architecture save enriched.json --repo r --json`
- **Expect:** the response carries `superseded`; `comemory show <old> --json`
  reports `superseded_by`; the tag lists both rows newest-first; `show`
  returns the new model.
- **Covered by:** `tests/cli__architecture.rs::a_second_save_supersedes_the_first`

### architecture-05 Show, Mermaid, and flag precedence

- **Flags:** `--format`, `--json`
- **Command:** `comemory architecture show --repo r --format mermaid`
- **Expect:** `flowchart LR`, one `subgraph` per group, one `-->|kind|` line
  per edge, byte-stable across runs; adding `--json` emits the model JSON
  instead. With no saved model, `show` exits 64 naming the repo.
- **Covered by:** `tests/cli__architecture.rs::show_renders_mermaid_and_the_global_json_flag_overrides_it`,
  `tests/cli__architecture_2.rs::show_and_check_name_the_repo_when_no_model_was_ever_saved`,
  `src/domains/architecture/tests/mermaid.rs`

### architecture-06 Check reports drift, never fails

- **Flags:** `--repo`, `--depth`, `--json`
- **Setup:** saved scaffold, then the second directory's only file deleted
  and a third directory added, re-indexed with `index-code --mode full`
- **Command:** `comemory architecture check --repo r --json`
- **Expect:** exit 0 with `drift_count: 0` on a fresh model; after the move,
  `stale_members` names `src/b`, `unmapped` names `src/c`, `drift_count >= 2`.
  Exit stays 0 — drift is a report, like `doctor`.
  Two mined kinds between one pair are one omission, reported once with the
  strongest weight.
- **Covered by:** `tests/cli__architecture_3.rs::a_freshly_saved_scaffold_reports_no_drift`,
  `tests/cli__architecture_3.rs::a_moved_file_shows_up_as_a_stale_member_and_an_unmapped_directory`,
  `tests/cli__architecture_3.rs::two_mined_kinds_between_one_pair_are_one_missing_edge`

### architecture-07 Learn runs the caller's agent

- **Flags:** `--command`, `--dry-run`, `--timeout`, `--json`
- **Setup:** real shell scripts in the test tmpdir standing in for the agent
- **Command:** `comemory architecture learn --repo r --command 'sh agent.sh {prompt_file}' --json`
- **Expect:** the prompt file is written under `<data_dir>/architecture/` and
  holds the scaffold; the agent's model is saved
  with `source: agent`. `--dry-run` writes the prompt and starts nothing. A
  template with no placeholder exits 64 before any spawn; a command exiting
  non-zero exits 70 with its stderr tail; prose with no JSON exits 65.
  `--timeout` kills and reaps the child.
- **Covered by:** `tests/cli__architecture_3.rs::learn_hands_the_scaffold_to_the_agent_and_saves_what_it_prints`,
  `tests/cli__architecture_3.rs::a_dry_run_writes_the_prompt_and_starts_nothing`,
  `tests/cli__architecture_3.rs::learn_refuses_a_template_without_a_placeholder_before_spawning`,
  `tests/cli__architecture_3.rs::a_failing_agent_surfaces_its_status_and_stderr_and_prose_is_a_data_error`

### architecture-08 The model stays out of everyday recall

- **Flags:** _(none)_
- **Command:** `comemory find "two" --json` before and after a save
- **Expect:** the same top-3 hit ids; the architecture memory is not among them.
- **Covered by:** `tests/cli__architecture_3.rs::storing_a_model_does_not_displace_everyday_recall`
