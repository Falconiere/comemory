# `domains/architecture/`

**What belongs here:** the component-level architecture model of an indexed
repository — the versioned value itself, the deterministic scaffold mined from
the code graph, the rules that decide whether a model may be saved, the read
that returns the current one, the drift check against today's index, the
Mermaid rendering, and the `learn` wrapper that hands a scaffold to an agent
command the caller named and validates whatever it prints.

**What does NOT belong here:** delivery. Clap flags, TTY lines, exit codes and
the `--format` switch stay in [`cli/`](../../cli/README.md)
(`cli/architecture.rs`, `cli/output/architecture.rs`). Persistence is
[`domains/memories/`](../memories/README.md)'s — a model is stored by calling
`memories::save::run`, never by writing a memory row here. The file-level
graph it is seeded from belongs to [`domains/graph/`](../graph/README.md);
this capability reads `build_code_graph` and adds no second query path. No
LLM runs here: `learn` spawns the caller's command and nothing else.

## Contents

| File | Primary item | Purpose |
| --- | --- | --- |
| `model.rs` | `Model` | The versioned `{ groups, components, edges }` value, its enums and its ceilings |
| `cluster.rs` | `key_for` | Directory-prefix keys, renderer-safe ids, README-seeded summaries, and the shared `covers` path test |
| `scaffold.rs` | `run` | The deterministic model: clusters over `build_code_graph` + `indexed_files`, with mined edges projected onto them |
| `validate.rs` | `validate` | Every save-time rule, checked against the indexed paths |
| `save.rs` | `run` | Validate, then store the model through `memories::save::run` with a supersede link |
| `current.rs` | `find` / `require` | The newest live memory tagged `architecture` and the fenced JSON inside it |
| `check.rs` | `run` | Drift: stale members, unmapped clusters, missing mined edges |
| `mermaid.rs` | `render` | Deterministic `flowchart` source |
| `prompt.rs` | `build` | The instructions + scaffold an agent is handed |
| `learn.rs` | `run` | Prompt file, one `sh -c` spawn of the caller's command under a deadline, then the validated save |
| `extract.rs` | `model_json` | The model dug out of an agent's stdout (fence, balanced object, or the raw text) |
