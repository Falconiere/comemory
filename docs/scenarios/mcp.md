# `comemory mcp`

Stdio Model Context Protocol server for agent hosts (Claude Code, Codex,
Cursor, Gemini CLI, Windsurf). Speaks JSON-RPC on stdin/stdout and offers a
curated eleven-tool catalog over the same command cores the CLI and
`comemory serve` call. Diagnostics go to stderr; stdout carries the protocol
and nothing else.

Repo scope is the **main worktree's basename** (`--repo`, else the label of
the process's working directory). A linked `git worktree` is never a
repository of its own: a session started inside one files memories under the
repo it belongs to, so the host, the shell wrapper and the status badge agree
on one key.

**Runnable tests:** `tests/cli_scenario_mcp.rs`, `tests/mcp__parity.rs`

**HTTP:** none — this command *is* a server (`transport: "cli-only"` in `GET /api/v1/commands`; asserted by `tests/api__parity.rs` and `tests/mcp__parity.rs`)

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).
`--json` is accepted and has no effect: the protocol owns stdout.

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--repo` | derived from the cwd | Default repo scope for every tool that accepts a `repo` parameter. An explicit `repo` on a call overrides it |
| `--read-only` | off | `save` and `feedback` answer a tool-level `read_only` error; tracked reads log no `retrieval_log` row |

## Scenarios

### mcp-01 Catalog and instructions

- **Flags:** `--read-only`
- **Setup:** throwaway `COMEMORY_DATA_DIR`, no corpus
- **Command:** `comemory mcp` (spawned by the rmcp client over child stdio)
- **Expect:** `initialize` succeeds; `tools/list` is exactly the eleven
  `comemory::mcp::catalog::TOOLS` names, each with its catalog description
  verbatim and an object `inputSchema`; `instructions` names `find` and
  `feedback`; `--read-only` lists the same eleven.
- **Covered by:** `tests/cli_scenario_mcp.rs::mcp_01_lists_catalog`

### mcp-02 Recall returns hits and a judgeable query id

- **Flags:** _(none)_
- **Setup:** two memories saved through the `save` tool
- **Command:** `tools/call find {"query":"postgres analytics","repo":"demo"}`
- **Expect:** the Postgres memory ranks first; the `query_id` is a real
  `retrieval_log` row (`recall_status` lists it as pending) and `feedback`
  acks it as `known_query`; an empty corpus returns zero hits and still a
  `query_id`.
- **Covered by:** `tests/cli_scenario_mcp.rs::mcp_02_find_returns_hits`

### mcp-03 Scope is the main worktree

- **Flags:** `--repo`
- **Setup:** a real `sample` repo with a linked `git worktree`
- **Command:** `comemory mcp` started with cwd inside the linked worktree,
  then a second server started in the main one
- **Expect:** `save` without `repo` files under `sample` and the second
  server's `find` sees it; in a plain tempdir an unscoped `save` is refused
  with `repo_required` and `find` still runs unscoped.
- **Covered by:** `tests/cli_scenario_mcp.rs::mcp_03_scope_is_main_worktree`

### mcp-04 Read-only refuses every writer

- **Flags:** `--read-only`
- **Setup:** one saved memory and one tracked recall on a writable session
- **Command:** `comemory mcp --read-only` over the same data dir
- **Expect:** `save` and `feedback` return `is_error` with structured
  `code: "read_only"` naming the tool; `find` still returns hits; `comemory
  recall-status --repo demo --json` reports the same `queries` count before
  and after the read-only recall.
- **Covered by:** `tests/cli_scenario_mcp.rs::mcp_04_read_only`

### mcp-05 Feedback provenance

- **Flags:** _(none)_
- **Setup:** a tracked `find`
- **Command:** `tools/call feedback {"query_id":…,"used":[id]}`, then the
  same call with `"confirmed_by_user": true`
- **Expect:** the `feedback_events` rows read back as `implicit` then
  `manual` (`store::feedback::events_for_query`), so an inferred verdict never
  enters the golden harvest; a malformed query id is a tool-level error with
  code `config`.
- **Covered by:** `tests/cli_scenario_mcp.rs::mcp_05_feedback_provenance`

### mcp-06 Parity with the clap surface

- **Flags:** _(none)_
- **Setup:** the live `Cli::command()` tree and a real `comemory serve`
- **Command:** walk every catalog entry against `Cli::command()` and probe
  every clap arg id against the tool's parameter type
- **Expect:** every catalog entry names a real subcommand; every clap arg id
  of each catalogued subcommand deserializes into that tool's parameter type;
  `mcp` reports `transport: "cli-only"` with empty routes and `recall-status`
  reports `http`.
- **Covered by:** `tests/mcp__parity.rs`

### mcp-07 Diagnostics stay off stdout

- **Flags:** _(none)_
- **Setup:** `RUST_LOG=info` in the child's environment
- **Command:** three raw JSON-RPC lines (`initialize`,
  `notifications/initialized`, `tools/list`) piped into `comemory mcp`
- **Expect:** exactly two JSON replies on stdout and nothing else; every log
  line lands on stderr, so a host that enables logging still parses the
  stream.
- **Covered by:** `tests/cli_scenario_mcp.rs::mcp_06_diagnostics_stay_off_stdout_under_rust_log`

### mcp-07 A rebuild between calls is visible to existing agents

- **Setup:** start a real MCP session and save a memory; rebuild using CLI.
- **Action:** a second session saves another memory, then the original session
  reads it and saves one of its own.
- **Expected:** both sessions see both new writes through the live database.
- **Covered by:** `mcp_07_rebuild_between_calls_keeps_agents_on_the_live_store`.

### mcp-08 Concurrent agents save without losing files or index rows

- **Setup:** four independent MCP processes over one store.
- **Action:** save distinct lessons concurrently, then replay one identical
  large lesson from every process.
- **Expected:** every save succeeds and is searchable; identical replays return
  one id, exactly one creation, and the complete body.
- **Covered by:** `mcp_08_concurrent_agents_save_and_find_each_others_memories`.
