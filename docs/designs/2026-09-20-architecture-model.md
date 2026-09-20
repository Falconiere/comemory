# Architecture model — Design

**Date:** 2026-09-20   **Status:** Approved   **Author:** Falconiere R. Barbosa
**Topic:** A per-repo architecture JSON — components, groups, typed edges —
seeded deterministically from the indexed code graph, enriched by an agent,
validated on save, and stored as one tagged memory so `console.comemory.io`
can draw it with any graph library (and the CLI can print Mermaid).

Brainstorm decision: 2026-09-20, on this feature branch.
Agent-integration direction this respects: comemory memory `490ef5f7`.
Console API shape this reuses: comemory memory `6f7baaf3`.

Reviewed 2026-09-20: two blockers fixed (the `install` flags in AC-14, the
missing `CLI_ONLY` registration), plus the output-writer, timeout,
redaction and multi-machine-freshness gaps named in that review.

## Problem

`comemory graph` already exports the *physical* graph: file nodes
(`file:<repo>:<path>`), `imports` / `co_changed` edges, PageRank weights
(`src/domains/graph/code_graph.rs`). Three costs follow from that being the
only graph:

1. **No semantic layer.** A 400-file repo renders as 400 nodes. Nothing in
   the store says "`src/domains/graph/` is the graph domain and the CLI calls
   it" — the abstraction a human calls *the architecture* exists only in
   directory READMEs and in whichever agent happens to have read them.
2. **Nothing for the console to draw.** `console.comemory.io` can list
   memories and code, but has no component-level model to render, and the
   file graph is the wrong altitude for an architecture picture.
3. **Agents re-derive it every session.** Each new session re-reads the tree
   to answer "how is this repo laid out", and that derivation is never
   written back anywhere the next session (or the console) can read.

## Non-Goals

1. **No new table, migration, or sync wire message.** The model rides the
   existing memory row and the existing memory push (`src/domains/sync/`).
2. **No new `/api/v1` route.** The console reads
   `GET /api/v1/memories?tag=architecture&repo=<r>` — an existing filter
   (`domains::memories::list::Request::tag`).
3. **No new MCP tool.** The curated eleven-tool catalog stays as decided in
   memory `490ef5f7`.
4. **No agent auto-detection and no default agent command.** `architecture
   learn` runs only the command template the caller passes on that
   invocation; comemory ships no command string and reads none from disk.
5. **No hook auto-invokes `learn`.** Nothing spends an agent's tokens without
   an explicit human command.
6. **No `config.toml` section in v1.** Every knob is a flag.
7. **No sequence diagrams, ER diagrams, runtime traces, or server-rendered
   images.** One component-level structural view; the renderer is the
   console's or Mermaid's problem.
8. **No non-zero exit on drift.** `check` is report-only like `doctor`; a
   gate that nobody armed is worse than no gate.

## Architecture

**Hybrid seed + enrich.** Three cores, one domain folder
`src/domains/architecture/`, delivered CLI-only (like `capture` and `sync`):

- **scaffold** — deterministic, no LLM. Reuses
  `domains::graph::query::build_code_graph(conn, repo, Rel::All, min_weight)`
  for nodes (path, `rank`, `symbols`) and edges. Files are clustered by
  repo-relative directory prefix truncated to `--depth` (default `2`), which
  is exactly the axis this codebase is organised on (`src/domains/*`,
  `src/cli/*`, `src/store/*`). A component's `rank` is the sum of its member
  file ranks; components are ordered by `rank` descending then id, and the
  list is truncated to `--max-components`. A file-level edge between two
  different components becomes a component edge whose `weight` is the number
  of contributing file edges; intra-component edges are dropped. Summaries
  are seeded from the first sentence of the cluster's `README.md` when one is
  indexed, else left empty for the agent to fill.
- **save** — validates a model against the *indexed* code — every `members`
  prefix must match at least one path from
  `store::indexed_files::list_for_repo(conn, repo)`, the existing read; no new
  store SQL and writes it
  through `domains::memories::save::run` with `kind: note`, `tags:
  ["architecture"]`, `repo: <r>`, `supersedes: [<previous model id>]`. The
  memory id is content-derived, so re-saving an unchanged model is a replay
  (same id, `created: false`) and creates no second row — the idempotency the
  refresh loop needs, for free.
- **show / check** — `show` reads the newest live memory tagged
  `architecture` for the repo (`memories::list` with `tag` + `repo`), parses
  the fenced JSON, and renders `json` or `mermaid`. `check` re-scaffolds and
  diffs the saved model against today's index.

**`learn` is a thin wrapper, not agent logic in the binary.** It scaffolds,
writes `<data_dir>/architecture/<repo>-prompt.md` (instructions + scaffold
JSON; the data directory rather than the world-writable temp dir, where a
predictable name is a symlink someone else can plant, and the repo label is
reduced to `[A-Za-z0-9._-]` so it cannot steer the write elsewhere),
substitutes `{prompt_file}` (or `{prompt}`, the prompt inline) into the
caller's `--command` template, runs it through `sh -c` on `tokio::process::Command` under
`tokio::time::timeout` (`std::process` cannot wait with a deadline; tokio is
already a dependency with `features = ["full"]`), extracts the model
from the child's stdout, and hands it to the same validated save path. The
binary holds a substitution and a process spawn; every word of agent guidance
lives in the bundled skill.

**Storage shape.** One live memory per repo:

```text
Architecture model — comemory

```json
{ "schema": 1, "repo": "comemory", ... }
```
```

Title line first (it is what `nav::title_of` reports), then the fenced JSON.
The console reads `tag=architecture`, takes the newest row, and parses the
first ```json fence.

**Reuse inventory.** `build_code_graph` (graph), `memories::save::run` +
`memories::list::run` (persistence), `git_utils::repo_label_at` (repo from
cwd), `cli::graph::Format`'s `ValueEnum` shape (`--format`),
`cli::output::*` (TTY/JSON writers), `domains::integrations::install::bundle`
(skill shipping), `utilities::context::Ctx` (connection + config).

## Interfaces / Schema

```text
comemory architecture scaffold [--repo R] [--depth N] [--max-components N]
                               [--min-edge-weight N] [--json]
comemory architecture save [FILE|-] [--repo R]
comemory architecture show  [--repo R] [--format json|mermaid]
comemory architecture check [--repo R] [--depth N] [--json]
comemory architecture learn --command TEMPLATE [--repo R] [--timeout SECS]
                            [--dry-run]
```

| Flag | Default | Effect |
| --- | --- | --- |
| `--repo` | cwd's git repo label | Repo scope for every subcommand |
| `--depth` | `2` | Directory-prefix depth a component clusters at (>= 1) |
| `--max-components` | `120` | Truncate after this many components, rank order (>= 1) |
| `--min-edge-weight` | `1` | Drop component edges below this weight (>= 1) |
| `--format` | `json` | `show` output: `json` \| `mermaid` |
| `--command` | *(required)* | `learn` agent template; must contain `{prompt_file}` or `{prompt}` |
| `--timeout` | `600` | `learn` child-process timeout, seconds |
| `--dry-run` | off | `learn` writes the prompt file and prints its path; spawns nothing |

**Transport registration.** `architecture` is CLI-only, so it joins
`CLI_ONLY` in `src/serve/routes/meta.rs`. Without that entry
`GET /api/v1/commands` reports it as `transport: "http"` and
`tests/cli_scenario_catalog.rs::every_command_documents_its_live_http_twin`
demands an `/api/v1` route it does not have. `docs/scenarios/architecture.md`
carries `**HTTP:** none — CLI-only (`transport: "cli-only"`)`, the same line
`docs/scenarios/capture.md` uses.

**Output writers.** Every subcommand owns both writers, as the rest of the
CLI does. TTY: `scaffold` prints `<repo> — <n> components, <m> edges` then one
line per component (`<rank>  <id>  <files> files  <name>`) and one per edge;
`save`, `check` and `learn` print their own summaries. `show` is format-driven
(`--format json|mermaid`) because a stored model is normally piped somewhere.
The global `--json` flag wins over `--format` exactly as it does for `comemory
graph` — `show --format mermaid --json` emits the model JSON, not Mermaid.

**Freshness under sync.** Two machines can each supersede the same predecessor
and push, leaving two live `architecture` rows for one repo. `show`, `check`
and the console all take the newest row by `created` (ties broken by id) and
ignore the rest; the next local save supersedes whichever it read.

Model (`schema: 1`, `serde(deny_unknown_fields)`):

```json
{
  "schema": 1,
  "repo": "comemory",
  "generated_at": "2026-09-20T12:00:00Z",
  "source": "scaffold",
  "direction": "LR",
  "groups":     [{ "id": "domains", "name": "Domain cores" }],
  "components": [{
    "id": "domains_graph", "name": "Graph", "group": "domains",
    "kind": "module", "members": ["src/domains/graph"],
    "summary": "Edges, PageRank, code graph.", "rank": 0.081, "files": 21
  }],
  "edges": [{ "from": "cli", "to": "domains_graph",
              "kind": "imports", "weight": 37 }]
}
```

Validation, all enforced by `save` (and therefore by `learn`):

| Rule | Violation |
| --- | --- |
| `schema == 1` | `Usage` (64), names the value found |
| `repo` non-empty and equal to the resolved scope | `Usage` (64) |
| `source` ∈ `scaffold` \| `agent` \| `manual` | `Usage` (64) |
| `direction` ∈ `LR` \| `TB` \| `RL` \| `BT` | `Usage` (64) |
| component `id` matches `^[A-Za-z][A-Za-z0-9_]{0,63}$`, unique | `Usage` (64) |
| `kind` ∈ `module` \| `service` \| `layer` \| `store` \| `external` | `Usage` (64) |
| `group` (when set) names a declared group | `Usage` (64) |
| `members` non-empty; every prefix matches ≥ 1 indexed file, except on an `external` component, which must have none | `Usage` (64), lists offenders |
| `summary` ≤ 280 chars | `Usage` (64) |
| edge `kind` ∈ `imports` \| `co_changed` \| `calls` \| `depends` \| `reads` \| `writes` \| `publishes` | `Usage` (64) |
| edge endpoints are declared components, `from != to` | `Usage` (64) |
| ≤ 200 components, ≤ 2000 edges, serialized model ≤ 32 KiB | `Usage` (64) |
| malformed JSON / unknown field | `Json` (65) |

`show --format mermaid` output (deterministic ordering, `"` escaped as `#quot;`):

```text
flowchart LR
  subgraph domains["Domain cores"]
    domains_graph["Graph"]
  end
  cli["CLI"]
  cli -->|imports| domains_graph
```

`check --json`:
`{ "repo": "r", "model_id": "8e1f…", "drift_count": 3, "stale_members": [{"component":"x","member":"src/gone"}], "unmapped": [{"path":"src/new","rank":0.04,"files":6}], "missing_edges": [{"from":"cli","to":"store","kinds":["imports","co_changed"],"weight":12}] }`

One pair of components is one `missing_edges` entry, carrying every mined kind
found between them and the strongest weight: `imports` and `co_changed` between
the same two directories is one omission, not two.

## Failure modes and edge cases

- **No indexed files for the repo** — `scaffold` / `check` exit 64 naming the
  repo and pointing at `comemory index-code`. No partial model is printed.
- **Repo unresolvable** (cwd outside a git repo, no `--repo`) — exit 64.
- **No saved model** — `show` / `check` exit 64 (`NotFound`) naming the repo.
- **Model `repo` ≠ resolved scope** — exit 64; never silently re-scoped.
- **Previous model trashed or already superseded** — `save` proceeds with an
  empty `supersedes` list rather than failing on a dangling id.
- **Unchanged re-save** — content-addressed replay: same id, `created:
  false`, no second row, `supersedes` self-reference skipped by
  `store::memory_row`.
- **Agent stdout with prose around the JSON** — `learn` takes the first
  ```json fence; absent a fence, the first balanced top-level object; absent
  both, exit 65 with the first 400 bytes of stdout echoed.
- **Agent command fails or times out** — exit 70 with the child's exit status
  and the last 400 bytes of its stderr. At `--timeout` the child is killed
  (`Child::kill`) and awaited to reap it, the error names the elapsed seconds,
  and nothing is saved.
- **Template without a placeholder** — exit 64 before any spawn.
- **Empty repo with one file** — scaffold emits one component, zero edges;
  `show --format mermaid` still emits a valid `flowchart` with one node.
- **Push-side redaction** — the model body goes through the same secret scan
  every memory does (`src/domains/sync/redact.rs`). A model holds repo-relative
  paths, component names and summaries, so it is expected to pass untouched; if
  a summary ever trips the scan the memory is skipped by sync exactly like any
  other, and the local model stays readable.
- **A deleted file after an incremental index** — `index-code` without
  `--mode full` only visits files that still exist, so a deleted file keeps its
  `indexed_files` row and its member stays valid until a full re-index. `check`
  therefore reports a stale member after `index-code --mode full`, and the
  bundled skill says so.
- **Concurrent saves** — serialized by the existing memory save path; the
  loser's model supersedes the winner's (last write wins), which is the same
  contract every `comemory save` already has.

## Acceptance criteria

- **AC-1:** On a real indexed git fixture with `src/a/one.rs` importing
  `src/b/two.rs`, `comemory architecture scaffold --repo r --json` emits
  `schema: 1`, one component per directory prefix at depth 2 whose `members`
  match indexed files, and one `imports` edge `a → b`; running it twice
  produces byte-identical output.
- **AC-2:** `--depth 1` on the same fixture collapses `src/a` and `src/b`
  into a single `src` component with both files as members and no edges
  (intra-component edges dropped).
- **AC-3:** `comemory architecture save model.json --repo r` writes one live
  memory with `kind: note`, `tags: ["architecture"]`, `repo: r`, and
  `comemory list --tag architecture --repo r --json` returns exactly that row.
- **AC-4:** Saving a changed model supersedes the first: `comemory show
  <old-id> --json` reports `superseded_by` = the new id, `list --tag
  architecture --repo r --json` returns both rows newest-first with the new id
  leading (a supersede annotates and demotes, it never deletes), and
  `architecture show` returns the new model.
- **AC-5:** A model whose component lists `members: ["src/does-not-exist"]`
  is refused with exit 64, the offending component id and path in stderr, and
  no memory written (`list --tag architecture` stays empty).
- **AC-6:** Each of duplicate component id, unknown `group`, edge endpoint
  absent from `components`, id `"1bad"`, `kind: "widget"`, and a model padded
  past 32 KiB is refused with exit 64 naming the offending value; an unknown
  JSON field is refused with exit 65.
- **AC-7:** `comemory architecture show --repo r --format mermaid` emits
  `flowchart LR`, one `subgraph` per declared group, one `-->|kind|` line per
  edge, `"` escaped, and is byte-identical across two runs.
- **AC-7b:** `show --repo r --format mermaid --json` emits the model JSON
  (parseable, `schema: 1`), not Mermaid — the same global-flag precedence
  `comemory graph --format dot --json` has.
- **AC-8:** `show` and `check` against a repo with no saved model exit 64 and
  name the repo in stderr.
- **AC-9:** After deleting `src/b/two.rs` from the fixture and re-indexing,
  `check --json` reports the `src/b` member under `stale_members`, a newly
  added indexed directory under `unmapped`, `drift_count >= 2`, and exits 0.
- **AC-10:** `scaffold | save` then `check --json` on the same index reports
  `drift_count: 0` and exits 0.
- **AC-11:** `comemory architecture learn --repo r --command 'sh
  fake-agent.sh {prompt_file}'`, where `fake-agent.sh` is a real script that
  reads the prompt file and prints an enriched model, saves that model: the
  live architecture memory body contains the agent's component names, and the
  prompt file it read contained the scaffold JSON.
- **AC-12:** `learn --dry-run --command 'sh marker.sh {prompt_file}'` prints
  the prompt file path, the file exists and contains the scaffold JSON, and
  the marker file `marker.sh` would have created does not exist.
- **AC-13:** `learn` exits 64 for a template with no placeholder (before any
  spawn, asserted by the same marker file), 70 when the command exits 3 (with
  its stderr tail echoed), and 65 when the command prints prose with no JSON.
- **AC-14:** `comemory install claude --config-dir <tmp>` extracts
  `plugins/comemory/skills/architecture-map/SKILL.md` byte-identical to
  `integrations/agent/skills/architecture-map/SKILL.md`, and that file
  contains the four literal commands of the refresh loop — `comemory
  architecture scaffold`, `comemory architecture save`, `comemory
  architecture check`, `comemory architecture show --format mermaid` — so an
  agent reading it has the whole save path without guessing flags.
- **AC-15:** `bash scripts/cli-docs-check.sh` exits 0 with
  `docs/cli-reference.md` carrying an `architecture` section that names all
  five subcommands, and `cargo nextest run -E 'test(cli_scenario_catalog)'`
  passes with `docs/scenarios/architecture.md` citing an existing
  `tests/<file>.rs::<fn>` for every flag in the table above.
- **AC-17:** `scaffold` without `--json` prints the component table (repo
  headline, one line per component, one per edge) and no JSON; `--depth 0`,
  `--max-components 0` and `--min-edge-weight 0` are clap usage errors (exit 2)
  rather than a model with nothing in it.
- **AC-18:** Two mined kinds between the same pair of modeled components are
  reported as one `missing_edges` entry naming both kinds and carrying the
  strongest weight, and the
  `learn` prompt is written under `<data_dir>/architecture/`, not the shared
  temp directory.
- **AC-16:** Saving a model does not displace normal recall: in the fixture
  repo, `comemory find "two" --json` returns the same ordered list of hit ids
  in its top 3 before and after the save, and the architecture memory's id is
  absent from that list.

## Acceptance evidence

| AC | Real input | Expected observable | Boundary / failure | Runnable check |
| --- | --- | --- | --- | --- |
| AC-1, AC-2 | `tests/common/git_repo.rs` fixture, two dirs, real `index-code` | scaffold JSON, stable bytes | single-file repo → one component, zero edges | `tests/cli__architecture.rs` |
| AC-3, AC-4 | scaffold output saved twice, second edited | first save is the only tagged row; after the second, `superseded_by` set and the new id leads | replayed identical model → `created:false` | `tests/cli__architecture.rs` |
| AC-5, AC-6 | hand-written invalid models under `tests/common/fixtures/architecture/` | exit 64/65, offender named, no row | 32 KiB+1 padded model | `src/domains/architecture/tests/validate.rs` + `tests/cli__architecture_2.rs` |
| AC-7, AC-7b | saved model with two groups | `flowchart LR` text, stable bytes | name containing `"` | `src/domains/architecture/tests/mermaid.rs` |
| AC-8 | fresh data dir, indexed repo, no model | exit 64, repo named | — | `tests/cli__architecture_2.rs` |
| AC-9, AC-10 | fixture re-indexed after a file delete + a new dir | `drift_count`, three drift lists | clean index → `0` | `tests/cli__architecture_3.rs` |
| AC-11–AC-13 | real `sh` scripts in the test tmpdir acting as the agent | model saved / prompt written / exit codes | nonzero child, prose stdout, no placeholder | `tests/cli__architecture_3.rs` |
| AC-14 | `comemory install claude --config-dir <tmp>` | byte-identical SKILL.md, four literal commands present | — | `src/domains/integrations/install/tests/bundle.rs` |
| AC-15 | built `Cli::command()` + docs | both gates green | — | `scripts/cli-docs-check.sh`, `tests/cli_scenario_catalog.rs` |
| AC-16 | fixture corpus before/after save | identical top-3 | — | `tests/cli__architecture_3.rs` |
| AC-17 | indexed fixture, no `--json` | table text, exit 2 on a zero knob | each of the three knobs | `tests/cli__architecture.rs`, `tests/cli__architecture_2.rs` |
| AC-18 | fixture mining both `imports` and `co_changed` between one pair | one drift entry; prompt path under the data dir | — | `tests/cli__architecture_3.rs` |

## Documentation impact

- `docs/scenarios/architecture.md` — new, every flag with a cited test.
- `docs/cli-reference.md` — regenerated (`scripts/cli-docs-check.sh`).
- `README.md`, `AGENTS.md` (Key Commands, Module Map) — new command family.
- `src/cli/README.md`, `src/domains/README.md` — new file index entries.
- `src/domains/architecture/README.md` — new folder index (guardrail).
- `integrations/agent/skills/architecture-map/SKILL.md` — new bundled skill:
  when to refresh the model, how to enrich a scaffold, how to save it.
- `docs/architecture.md` — one paragraph on the model and its storage shape.

## Open Questions

1. **Cluster depth default (2).** Right for this repo's `src/<area>/<module>`
   layout; a flat repo may want 1. Non-blocking — it is a flag.
2. **`[architecture] agent_command` config key.** Deliberately out of v1
   (Non-Goal 6); revisit once `learn` has real usage. Non-blocking.
3. **Console rendering library.** Owned by the console codebase; the model
   is library-agnostic by construction. Non-blocking.
