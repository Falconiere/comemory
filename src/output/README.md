# output/

**What belongs here:** the TTY (`owo-colors`) and JSON (`serde_json`)
rendering shared by every subcommand — one file per command's output shape,
plus the generic pagination envelope and the shared color/line helpers both
modes route through.

**What does NOT belong here:** computing what to render. `output/` only
formats data already assembled by `cli::*` and `retrieval::*`; it never
queries the store or the pipeline itself.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `consolidate.rs` | `emit` | Rendering for `comemory consolidate` (cluster blocks + keeper marker) |
| `context.rs` | `Envelope` | Rendering for `comemory context` (headline bundle) |
| `edges.rs` | `Row` | Rendering for `comemory edges` (triplet rows + shared page envelope) |
| `graph.rs` | `Node` | Rendering for `comemory graph` (JSON / DOT / HTML relation-graph export) |
| `graph_template.html` | — | HTML template the `graph.rs` HTML renderer fills in |
| `json.rs` | `write` | Single-line JSON writer shared by every `--json` surface |
| `page.rs` | `Page` | Generic pagination envelope shared by every paged command |
| `prune.rs` | `emit` | Rendering for `comemory prune` (candidate lists) |
| `search.rs` | `Row` | Rendering for `comemory search` (memory hits) |
| `search_code.rs` | `Row` | Rendering for `comemory search-code` (code hits) |
| `tty.rs` | `write_header` | Colored line builders and the shared page footer |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/output.rs` (`pub mod
<name>;`) and callers import concrete paths.
