# cli/output/

**What belongs here:** the TTY (`owo-colors`) and JSON (`serde_json`)
rendering shared by every subcommand — one file per command's output shape,
plus the shared color/line helpers both modes route through. The generic
pagination envelope moved to `utilities::pagination` and each command's result
model to the capability that produces it, with #166. #171 took the rest of
the retrieval shapes: the `search` / `search-code` / `context` `--json`
envelopes and their rows are what both transports serialize, so they moved to
`domains::retrieval::{search_result, code_search_result, context_result,
scope}` and the explain strip to `domains::retrieval::explain`; the three
files here keep `emit` and `write_tty`. #178 moved the folder itself under
`cli/`, where it always belonged, and took `edges`'s `Row` and `envelope` with
it to `domains::graph::edges_result` — the `/api/v1/edges` handler was building
its response body from this folder, which was the last place HTTP depended on
CLI presentation. `comemory::output::*` still resolves for library consumers,
through the crate-root alias in `lib.rs`.

**What does NOT belong here:** computing what to render. `cli/output/` only
formats data already assembled by `cli::*` and the capability modules; it never
queries the store or the pipeline itself. Nor anything the HTTP surface needs:
a shape both transports serialize belongs to the capability that produces it,
which is why every `envelope` builder now lives under `domains::`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `architecture.rs` | `write_model` | Rendering for the `comemory architecture` family (component table, Mermaid source, save result, drift report) over `domains::architecture`'s values |
| `consolidate.rs` | `emit` | Rendering for `comemory consolidate` (cluster blocks + keeper marker) |
| `context.rs` | `emit` | Rendering for `comemory context` (headline bundle) over `domains::retrieval::context_result`'s envelope |
| `edges.rs` | `emit` | Rendering for `comemory edges` (triplet rows + the pagination footer) over `domains::graph::edges_result`'s envelope |
| `graph.rs` | `to_dot` | Rendering for `comemory graph` (JSON / DOT / HTML relation-graph export) over `domains::graph::code_graph`'s model |
| `graph_template.html` | — | HTML template the `graph.rs` HTML renderer fills in |
| `json.rs` | `write` | Single-line JSON writer shared by every `--json` surface |
| `prune.rs` | `emit` | Rendering for `comemory prune` (candidate lists) |
| `search.rs` | `emit` | Rendering for `comemory search` (memory hits) over `domains::retrieval::search_result`'s envelope |
| `search_code.rs` | `emit` | Rendering for `comemory search-code` (code hits) over `domains::retrieval::code_search_result`'s envelope |
| `tty.rs` | `write_header` | Colored line builders and the shared page footer |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/cli/output.rs` (`pub mod
<name>;`) and callers import concrete paths.
