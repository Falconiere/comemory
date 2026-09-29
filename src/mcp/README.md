# mcp/

**What belongs here:** the `comemory mcp` stdio adapter — the third delivery
surface beside `cli` and `serve`. The curated tool catalog, the per-session
state (session mutex, paths, config, default repo scope, `--read-only`),
the blocking-pool bridge every tool body runs its core through, the
crate-error-to-protocol-result mapping, and the one parameter type MCP owns
(`feedback`, whose provenance rule differs from the core's default, plus the
architecture tool shapes for nested CLI commands).

**What does NOT belong here:** command logic. Every tool calls a
`domains::<capability>::<cmd>::run` core — the same one the CLI and the HTTP
routes call — and never reimplements ranking, scoring or storage. Nothing here
imports `cli` or `serve`, and no domain, `config` or `utilities` file may
import this folder (owner `delivery::mcp` in
`scripts/architecture-policy.json`). Transport-neutral pieces two adapters
share live in `utilities`: `run_blocking` in `utilities::blocking` and the
`Error → (code, Class)` map in `utilities::error_code`.

**stdout belongs to the protocol.** Only JSON-RPC frames may be written there;
diagnostics go to stderr through `tracing`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `catalog.rs` | `TOOLS` | The eighteen-row tool table — name, the CLI command path whose core it runs, whether it writes, and the agent-facing description the tool attribute reuses verbatim |
| `exec.rs` | `run`, `Access` | Run a core on the blocking pool with the session locked and a fresh database connection opened/dropped inside that task; an `Access::Write` on a `--read-only` session is refused before the closure is ever built |
| `params.rs` | `FeedbackParams` | Adapter-owned feedback provenance, typed architecture scaffold, save and show parameters, and `ProjectShowParams`, whose `view` (`charter` default, `plan`, `activity` with `limit`/`cursor`/`order`, or `evidence` with `limit`/`cursor`/`kind`/`trust`/`workItemId`, each field refused on a view that does not read it) picks the core `project_show` runs |
| `result.rs` | `into_tool_result` | Success as structured content; every non-`Internal` error class as a tool-level `{code, message}`; `Internal` as a protocol error carrying only the code word. Plus the two adapter-raised refusals, `read_only` and `repo_required` |
| `scope.rs` | `default_repo` | The session's default repo scope (`--repo`, else the cwd's main-worktree label) and `resolve`, the rule that lets an explicit non-empty parameter win |
| `server.rs` | `ComemoryServer` | The rmcp service object: `read_router() + project_router() + project_write_router() + write_router()`, the session state every tool body clones, and `get_info` — tools capability plus the five-line loop `instructions`, with the missing-scope sentence appended when no default repo resolved |
| `state.rs` | `McpState` | Cheaply-cloneable per-session state: `Arc<Mutex<()>>`, per-call connections, paths, config, default repo, `read_only`, the `track()` decision for tracked reads, and `project_envelope()`, the local agent every project tool runs as |
| `tools_projects.rs` | `project_router` | The project tools: the readers `project_list` and `project_show` — the `domains::projects` list core, and the show, plan, activity-page or evidence-page core by `project_show`'s `view` (each read of one project is a view, not a catalog row: #335, #331, #346) , with no repo scope, run under the session's local-agent envelope (`McpState::project_envelope`: all six capabilities, no human verb) |
| `tools_project_writes.rs` | `project_write_router` | The project writer tools: `project_evidence` (`domains::projects::evidence_add`, #346), refused `read_only` on a `--read-only` session through `tools_write::write_tool`, with no repo scope, run under the session's local-agent envelope |
| `tools_project_writes.rs` | `project_write_router` | The project writer tools: `project_propose` (#336), the `domains::projects::propose` core, which submits a plan proposal for human review; refused in a `--read-only` session, run under the session's local-agent envelope |
| `tools_projects.rs` | `project_router` | The two project read tools, `project_list` and `project_show` — the `domains::projects` list core, and the show, plan or activity-page core by `project_show`'s `view` (each read of one project is a view, not a catalog row: #335, #331), with no repo scope, run under the session's local-agent envelope (`McpState::project_envelope`: all six capabilities, no human verb) |
| `tools_read.rs` | `read_router` | Twelve read tools: recall/repository tools plus `architecture_scaffold`, `architecture_show` and `architecture_check`, all of which require a resolved repo scope |
| `tools_write.rs` | `write_router` | Three write tools: `save`, `feedback`, and `architecture_save`; both save tools refuse an unscoped call before the store is touched. `write_tool`, the read-only-gated runner, is shared with `project_evidence` |

The `comemory mcp` subcommand itself is `src/cli/mcp.rs` (flags and data-dir
resolution are a CLI concern); `mcp::serve` in `src/mcp.rs` is what it calls.
Crate-root journey tests that drive a real `comemory mcp` process live in
`tests/`, never under `src/mcp/`.

Idle MCP sessions hold no database connection, so a rebuild between calls is
visible on the next call. Initial opens share the store setup lock and saves
reserve SQLite's writer before mirror reads. The catalog recommends `find(k=3)`
plus selected `show` calls; `context` has no token budget.

Architecture model tools are deliberately MCP and CLI only: the console reads
the saved tagged memory rather than an architecture HTTP route. Use
`architecture_scaffold`, enrich its returned `model`, `architecture_save`, then
`architecture_check`; `architecture_show` returns the model JSON by default or
`{ "mermaid": "..." }` when `format` is `mermaid`. Every architecture tool
requires a repo from its parameter or the session default. `architecture_save`
is a write and returns `read_only` before checking scope in a read-only session;
otherwise an unscoped call returns `repo_required`. The serialized model is capped
at 32 KiB before domain-schema deserialization; the stdio transport parses the
JSON-RPC message first, with serde_json's default nesting guard. No MCP tool
exposes `architecture learn` or the command execution behind it.
