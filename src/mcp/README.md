# mcp/

**What belongs here:** the `comemory mcp` stdio adapter — the third delivery
surface beside `cli` and `serve`. The curated tool catalog, the per-session
state (shared connection, paths, config, default repo scope, `--read-only`),
the blocking-pool bridge every tool body runs its core through, the
crate-error-to-protocol-result mapping, and the one parameter type MCP owns
(`feedback`, whose provenance rule differs from the core's default).

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
| `catalog.rs` | `TOOLS` | The eleven-row tool table — name, the clap subcommand whose core it runs, whether it writes, and the agent-facing description the tool attribute reuses verbatim |
| `exec.rs` | `run`, `Access` | Run a core on the blocking pool with the connection locked only inside that task; an `Access::Write` on a `--read-only` session is refused before the closure is ever built |
| `params.rs` | `FeedbackParams` | The `feedback` tool's parameters: the core's request with `source` replaced by `confirmed_by_user`, so an inferred verdict lands as `implicit` and only a stated one as `manual` |
| `result.rs` | `into_tool_result` | Success as structured content; every non-`Internal` error class as a tool-level `{code, message}`; `Internal` as a protocol error carrying only the code word. Plus the two adapter-raised refusals, `read_only` and `repo_required` |
| `scope.rs` | `default_repo` | The session's default repo scope (`--repo`, else the cwd's main-worktree label) and `resolve`, the rule that lets an explicit non-empty parameter win |
| `server.rs` | `ComemoryServer` | The rmcp service object: `read_router() + write_router()`, the session state every tool body clones, and `get_info` — tools capability plus the five-line loop `instructions`, with the missing-scope sentence appended when no default repo resolved |
| `state.rs` | `McpState` | Cheaply-cloneable per-session state: `Arc<Mutex<Connection>>`, paths, config, default repo, `read_only`, root overrides, and the `track()` decision for tracked reads |
| `tools_read.rs` | `read_router` | The nine read tools (`find`, `search`, `search_code`, `context`, `show`, `list`, `edges`, `repos`, `recall_status`): resolve scope, run the core, return the object the matching `/api/v1` route puts in its envelope's `data` |
| `tools_write.rs` | `write_router` | The two write tools: `save`, which refuses an unscoped write with `repo_required` before the store is touched, and `feedback`, which maps `FeedbackParams` onto the core's request |

The `comemory mcp` subcommand itself is `src/cli/mcp.rs` (flags and data-dir
resolution are a CLI concern); `mcp::serve` in `src/mcp.rs` is what it calls.
Crate-root journey tests that drive a real `comemory mcp` process live in
`tests/`, never under `src/mcp/`.
