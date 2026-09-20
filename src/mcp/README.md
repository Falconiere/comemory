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
| `catalog.rs` | `TOOLS` | The fifteen-row tool table — name, the CLI command path whose core it runs, whether it writes, and the agent-facing description the tool attribute reuses verbatim |
| `exec.rs` | `run`, `Access` | Run a core on the blocking pool with the session locked and a fresh database connection opened/dropped inside that task; an `Access::Write` on a `--read-only` session is refused before the closure is ever built |
| `params.rs` | `FeedbackParams` | Adapter-owned feedback provenance plus typed architecture scaffold, save and show parameters |
| `result.rs` | `into_tool_result` | Success as structured content; every non-`Internal` error class as a tool-level `{code, message}`; `Internal` as a protocol error carrying only the code word. Plus the two adapter-raised refusals, `read_only` and `repo_required` |
| `scope.rs` | `default_repo` | The session's default repo scope (`--repo`, else the cwd's main-worktree label) and `resolve`, the rule that lets an explicit non-empty parameter win |
| `server.rs` | `ComemoryServer` | The rmcp service object: `read_router() + write_router()`, the session state every tool body clones, and `get_info` — tools capability plus the five-line loop `instructions`, with the missing-scope sentence appended when no default repo resolved |
| `state.rs` | `McpState` | Cheaply-cloneable per-session state: `Arc<Mutex<()>>`, per-call connections, paths, config, default repo, `read_only`, and the `track()` decision for tracked reads |
| `tools_read.rs` | `read_router` | Twelve read tools: recall/repository tools plus `architecture_scaffold`, `architecture_show` and `architecture_check`, all of which require a resolved repo scope |
| `tools_write.rs` | `write_router` | Three write tools: `save`, `feedback`, and `architecture_save`; both save tools refuse an unscoped call before the store is touched |

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
is a write and obeys `--read-only`; no MCP tool exposes `architecture learn` or
the command execution behind it.
