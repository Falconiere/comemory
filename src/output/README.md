# output/

**What belongs here:** the TTY (`owo-colors`) and JSON (`serde_json`)
rendering shared by every subcommand — one file per command's output shape,
plus the shared color/line helpers both modes route through. The generic
pagination envelope moved to `utilities::pagination` and each command's result
model to the capability that produces it, with #166. #171 took the rest of
the retrieval shapes: the `search` / `search-code` / `context` `--json`
envelopes and their rows are what both transports serialize, so they moved to
`domains::retrieval::{search_result, code_search_result, context_result,
scope}` and the explain strip to `domains::retrieval::explain`; the three
files here keep `emit` and `write_tty`.

**What does NOT belong here:** computing what to render. `output/` only
formats data already assembled by `cli::*`, `api::*` and the capability
modules; it never queries the store or the pipeline itself.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `consolidate.rs` | `emit` | Rendering for `comemory consolidate` (cluster blocks + keeper marker) |
| `context.rs` | `emit` | Rendering for `comemory context` (headline bundle) over `domains::retrieval::context_result`'s envelope |
| `edges.rs` | `Row` | Rendering for `comemory edges` (triplet rows + shared page envelope) |
| `graph.rs` | `to_dot` | Rendering for `comemory graph` (JSON / DOT / HTML relation-graph export) over `domains::graph::code_graph`'s model |
| `graph_template.html` | — | HTML template the `graph.rs` HTML renderer fills in |
| `json.rs` | `write` | Single-line JSON writer shared by every `--json` surface |
| `prune.rs` | `emit` | Rendering for `comemory prune` (candidate lists) |
| `search.rs` | `emit` | Rendering for `comemory search` (memory hits) over `domains::retrieval::search_result`'s envelope |
| `search_code.rs` | `emit` | Rendering for `comemory search-code` (code hits) over `domains::retrieval::code_search_result`'s envelope |
| `tty.rs` | `write_header` | Colored line builders and the shared page footer |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/output.rs` (`pub mod
<name>;`) and callers import concrete paths.
