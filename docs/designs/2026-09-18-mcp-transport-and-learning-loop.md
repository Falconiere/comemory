# MCP stdio adapter and learning-loop hooks — Design

**Date:** 2026-09-18   **Status:** Approved   **Author:** Falconiere R. Barbosa
**Topic:** A third delivery adapter, `comemory mcp`, over the same command
cores `cli` and `serve` call, plus the bundled skills and hooks that make an
agent seed memory, judge what it recalls, and close the feedback loop.

Brainstorm decision: comemory memory `490ef5f7`. Provenance finding: memory
`cad62a9b`. Ownership decision this builds on: memory `2a74a6db`.

## Problem

An agent reaches comemory today through a 304-line bash wrapper that a
SessionStart hook publishes as a symlink, and only from a `Bash` tool call.
Three costs follow:

1. **Hosts.** Only Claude Code and Codex are covered, and only through shell.
   Every MCP-capable host (Claude Code, Codex, Cursor, Gemini CLI, Windsurf)
   could call comemory as native tools with typed schemas; none can.
2. **Surface.** The wrapper dispatches 18 verbs and omits `find`, the one
   cross-domain ranked search, plus `show` and `edges`. Repo scope is enforced
   by a 217-line `PreToolUse` deny hook that parses shell with `python3`.
3. **Loop.** The learning loop is advisory text. `UserPromptSubmit` injects a
   reminder every turn and recalls nothing. Nothing checks whether a recall
   was ever judged. And a verdict an agent records through the wrapper lands
   as `manual` provenance, which `eval` and `mine` harvest as human ground
   truth (`src/domains/learning/feedback.rs`, #130). A fresh repo starts empty
   with no bootstrap path.

## Non-Goals

1. **No streamable-HTTP MCP transport.** stdio only. `serve` is untouched
   except for the `run_blocking` relocation in § Architecture.
2. **No MCP resources or prompts.** Tools only.
3. **No model reranking.** #213 wires the reranker into retrieval. Skill text
   may describe agent-side judging now and model-side reranking only after
   #213 ships a flag.
4. **No renames** of `TOOLU_COMEMORY_REPO`, `MY_CLAUDE_COMEMORY_REPO`, the
   `TOOLU_*` config env vars, or the `.toolu/skills/` directory.
5. **No hook logic moves into the binary.** Hooks stay bash; the binary gains
   one read-only query they call.
6. **No plugin bundles for other hosts.** Cursor, Gemini CLI and Windsurf
   register `comemory mcp` by hand; the guide shows the one-line config.
7. **No in-process LLM and no network** from the MCP server.
8. **No schema migration.** `recall-status` is a query over the existing
   `retrieval_log` and `feedback_events` tables.
9. **No Windows.** The supported platforms are the three cargo-dist targets.
10. **No change to the `/api/v1` route set or envelope** beyond the one added
    `GET /learning/recall-status` route.
11. **No `delete` tool.** `save --supersedes` is the sanctioned correction
    path; deletion stays a CLI verb.

## Architecture

### The adapter

`src/mcp.rs` + `src/mcp/` is a delivery adapter, a sibling of `cli` and
`serve`. It imports `domains::*`, `utilities::*`, `config` and
`store::Connection`, never `cli` or `serve`. No domain, `config` or
`utilities` file may import it.

A third adapter is a new owner, `delivery::mcp`, and the gates hardcode the
current two in seven places. Every one of them changes in this work, and
AC-14 names a flip-and-revert proof for each:

1. `scripts/architecture-policy.json`: `mcp` joins `staged_top_level_dirs`
   and `staged_root_modules`.
2. `scripts/lib/architecture-inventory.sh`: the two literal arrays that
   `validate_inventory` compares the policy against (lines 27–28), the owner
   vocabulary regex (line 31) becomes `delivery::(cli|serve|mcp)`, and the
   `expected` map gains `elif . == "src/mcp.rs" or startswith("src/mcp/")
   then "delivery::mcp"` before the `shared::root` fallback.
3. `scripts/architecture-check.sh`: the owner regexes at lines 62 and 102,
   and the delivery-dependency regex at line 413 becomes
   `^crate::(cli|serve|mcp)(::|$)` so a domain importing `mcp` fails as a
   `delivery dependency`.
4. `scripts/test-architecture-policy.sh`: its negative cases keep passing
   because they mutate the live policy; a new case flips a `src/mcp/` row's
   owner to `delivery::cli` and expects `capability ownership mismatch`.
5. `docs/designs/2026-09-17-domain-first-migration-inventory.md`: one row per
   new production file in the seven-column shape
   `| path | public | bridge | assets | owner | target | issue |`:
   `path` and `target` are the file itself (`target` is regex-checked and
   globally unique); `public` is `comemory::<module path>; preserve` for the
   modules crate-root tests reach (`mcp`, `mcp::catalog`, `mcp::params`,
   `domains::learning::recall_status`, `utilities::error_code`) and
   `private` for the rest; `bridge` is the colocated test file under the
   folder's `tests/` or `none`; `assets` is `none`; `owner` is
   `delivery::mcp` under `src/mcp*`, else the folder's owner
   (`delivery::cli`, `shared::utilities`, `shared::config`,
   `domains::learning`, `infrastructure::store`, `delivery::serve`); `issue`
   is `retain`, the value every post-migration file carries (inventory line
   835, `src/utilities/rerank_runner.rs`).
6. `guardrails.config.json`: `src.topLevel` gains `mcp`; `src.requireReadme`
   gains `mcp` (entries are `src`-relative: `cli`, `serve`, `domains/code`).
   `src/mcp/` holds flat files plus `tests/`, which the `"*": ["tests", …]`
   nesting rule already allows.
7. `src/serve/routes/meta.rs` `CLI_ONLY` gains `mcp`, and so does its
   deliberately independent mirror in `tests/api__parity.rs`.

| File | Primary item | Purpose |
| --- | --- | --- |
| `src/mcp.rs` | `McpOptions`, `serve` | Open the store, build `McpState`, run the rmcp service over stdio until EOF |
| `src/mcp/state.rs` | `McpState` | `Arc<Mutex<()>> session gate + per-call connection` + `Arc<Paths>` + `Arc<Config>` + default repo + `read_only`; the `AppState` shape without token, port, roots or jobs |
| `src/mcp/catalog.rs` | `ToolEntry`, `TOOLS` | Static table `(name, command, mutating)` — the read-only gate and the parity test read it |
| `src/mcp/server.rs` | `ComemoryServer` | `#[tool_handler] impl ServerHandler`: `get_info` with `instructions`, composes the two routers |
| `src/mcp/tools_read.rs` | `read_router` | `#[tool_router(router = read_router)]`: `find`, `search`, `search_code`, `context`, `show`, `list`, `edges`, `repos`, `recall_status` |
| `src/mcp/tools_write.rs` | `write_router` | `#[tool_router(router = write_router)]`: `save`, `feedback` |
| `src/mcp/exec.rs` | `run`, `Access` | Clone state, `run_blocking`, lock the session and open/drop the current connection for the synchronous call, build `Ctx::borrowed`, run the core; an `Access::Write` on a read-only session is refused before the closure exists |
| `src/mcp/result.rs` | `into_tool_result` | `utilities::error_code::classify` → tool-level `structured_error` for every class but `Internal`, protocol `ErrorData` for `Internal` (§ Failure modes) |
| `src/utilities/error_code.rs` | `classify`, `Class` | The code-word half of today's `serve::envelope::status_and_code`, extracted so `serve` and `mcp` share one total mapping; `envelope` keeps only `Class → StatusCode` |
| `src/mcp/scope.rs` | `default_repo` | `--repo` flag, else `domains::code::git_utils::repo_label_at(cwd)`, else `None` |
| `src/mcp/params.rs` | `FeedbackParams` | The one MCP-local parameter type; every other tool takes the core's `Request` |
| `src/cli/mcp.rs` | `Args` | clap shape: `--repo`, `--read-only` (no `--root`: no catalogued core resolves a repo root) |
| `src/utilities/blocking.rs` | `run_blocking` | Moved out of `serve::routes` (`pub(crate) async fn run_blocking<T, F>(f: F) -> Result<T>`, `tokio::task::spawn_blocking` plus `Error::Other`, no axum) so both adapters share it (Binding Rule 1). Roughly 25 import sites under `src/serve/routes/` repoint; `serve::routes` keeps no re-export |
| `src/config/env.rs` | `access_tracking_enabled` | `cli::track_searches` moves here unchanged; `cli` and `serve` call the new path directly, no re-export (Binding Rule 2) |

Decisive trade-off: **a curated catalog of eleven tools, not the 94-route
table.** An agent host budgets tool descriptions into every turn; the route
table exists for consoles. Parity is bought back the way `tests/api__parity.rs`
buys it for HTTP: every catalog entry names a real clap subcommand and its
parameter type must deserialize every one of that subcommand's argument ids.

Reuse: `Ctx::borrowed` and the mutex-per-connection pattern from
`serve::AppState`; `domains::code::git_utils::repo_label_at` as the Rust twin
of the wrapper's `comemory_repo_key` (both are the main worktree's basename,
so MCP, wrapper and status badge agree on one key);
`config::env::env_parse` for the `COMEMORY_DISABLE_ACCESS_TRACKING` test hook,
which moves from `cli::track_searches` into
`config::env::access_tracking_enabled` so `cli`, `serve` and `mcp` read one
definition.

Parameter schemas come from `schemars::JsonSchema` derived on the domain
`Request` types the catalog names, plus `memories::frontmatter::Kind` and
`memories::list::Sort`. One source of truth; serde attributes are honored.
`schemars` is pinned to the `^1` line rmcp 3.4 requires. The derive is
invisible to the layer check, which keeps only `crate::`-rooted paths
(`scripts/architecture-check.sh` line 314), so it is not a layer edge.

### Bundle payload

`src/domains/integrations/install/bundle.rs` `FILES` is the exhaustive embed
list; a file absent from it never reaches an installed host. Two entries are
added: `hooks/session-end.sh` and `skills/memory-bootstrap/SKILL.md`. The
edited `hooks/hooks.json`, `hooks/memory-lifecycle.sh`,
`hooks/session-start.sh`, `hooks/comemory-status.sh` and
`skills/agent-memory/SKILL.md` are already listed. `.mcp.json` is
deliberately not embedded (§ Host registration).

Dependencies: `rmcp = "3.4"` with `default-features = false, features =
["server", "transport-io", "macros"]` (Apache-2.0; MSRV 1.88 under the crate's
1.95), `schemars = "1"` (MIT). Dev: `rmcp` with `client` and
`transport-child-process` so the journey test drives the real binary with the
real client. `deny.toml` already allows both licenses.

### Host registration

`comemory install <host>` writes `<bundle>/plugins/comemory/.mcp.json` — the
plugin root both Claude Code and Codex read — on every install, with the
absolute path of the installing binary. It is a per-install artifact like the
two marketplace catalogs (`write_catalogs`), written temp-then-rename and
never part of the embedded-bytes equality check, so moving the binary and
re-running install refreshes the path without a `bundle differs` refusal.
`--dry-run` reports the path and writes nothing.

`hooks.json` keeps `scope.sh` for raw `Bash` calls; MCP calls never pass
through it. `session-start.sh` keeps publishing the wrapper as the fallback
for hosts without MCP and for `maintain`.

### The loop

Four hook changes and two skills, all inside `integrations/agent/`:

- **Recall injection.** `memory-lifecycle.sh` on `UserPromptSubmit` runs
  `comemory find` on the prompt (memory domain, `k` from config, tracking
  disabled so the hint mints no `retrieval_log` row) and injects ids and
  titles only. Agents can `show` selected hints without repeating the search;
  an explicit `find` call is tracked. Skipped for
  prompts under `recall.injectMinChars`, for the existing skip list, and when
  the call exceeds five seconds with `timeout`/`gtimeout` available.
- **Bootstrap nudge.** `comemory-status.sh` already counts this repo's
  memories; when the count is zero it now also emits `additionalContext`
  pointing at the `memory-bootstrap` skill.
- **Session marker.** `session-start.sh` writes
  `<cfg>/comemory/session-<session_id>.start` holding the UTC RFC 3339 start
  instant, **write-once** (`[ -e "$f" ] || printf … >"$f"`): SessionStart
  also fires on `resume`, `clear` and `compact` with the same `session_id`,
  and rewriting the marker would move `--since` forward past unjudged
  recalls. The Stop branch reads it as the `--since` bound. Markers older
  than seven days are swept the way `maintain-*` directories are.
- **Advisory reminder.** `memory-lifecycle.sh` on `Stop` calls
  `comemory recall-status --repo KEY --since <session start>`. This window is
  shared across agents, so it cannot authorize a per-session block. The hook
  emits a compact `systemMessage` at most once per session, never a block,
  and only names pending rows with recorded ids. Empty recalls need no
  fabricated feedback/save. `recall.enforce` remains the compatibility switch.
  Errors, missing markers and `stop_hook_active` stay silent. The daily local
  maintenance latch remains independent.
- **SessionEnd capture.** `hooks.json` registers `hooks/session-end.sh` for
  both hosts. It starts detached `comemory capture session --from-hook` only
  for Claude Code and exits silently under Codex, whose transcript format
  has no capture/distill adapter yet. `comemory capture install-hook` stays
  for non-plugin Claude setups.
- **`agent-memory/SKILL.md`** leads with the MCP tools, keeps the wrapper as
  fallback, and states the loop: recall, judge every hit you acted on, record
  `feedback` (implicit unless the user confirmed), save with evidence.
- **`memory-bootstrap/SKILL.md`** (new): when the repo has no memories, index
  code, index docs, distill past transcripts, then save the decisions the repo
  cannot derive from itself. Not copies of documentation.

### The one new core

`domains::learning::recall_status` — read-only, no confirm gate. Given a repo
and a lower time bound it reports tracked queries, verdicts, saves and the
queries still awaiting a verdict. Three adapters call it: CLI
`comemory recall-status`, HTTP `GET /api/v1/learning/recall-status`, and the
MCP tool `recall_status`. Its SQL is two new store functions, each returning
owned data across the boundary: `store::retrieval_log::pending_since(conn,
repo, since) -> Result<Vec<PendingRow>>` (hand SQL, a `LEFT JOIN
feedback_events … IS NULL`, tracked in `docs/guides/runtime-orm.md`) and
`store::memory_row::count_created_since(conn, repo, since) -> Result<u64>`.
The created-since bound is a new predicate, so it is a new function rather
than a `stats_counts::Corpus` variant, which exists precisely so no predicate
crosses the store boundary. The verdict count reuses
`store::feedback::event_counts`-style reads with the same window.

## Interfaces / Schema

### CLI

```text
comemory mcp [--repo NAME] [--read-only]
comemory recall-status [--repo NAME] [--since RFC3339] [--json]
```

`comemory mcp` writes nothing but JSON-RPC to stdout; diagnostics go to
stderr through `tracing`. The global `--json` flag is accepted and has no
effect. `--data-dir` applies as everywhere. Exit `0` on stdin EOF; a store
that cannot be opened propagates through the `main.rs` mapping (`EX_SOFTWARE`
70 for `Error::Sqlite`, `EX_IOERR` 74 for `Error::Io`).

### Tool catalog (`src/mcp/catalog.rs`)

| Tool | Command | Mutating | Parameters |
| --- | --- | --- | --- |
| `find` | `find` | no | `retrieval::find::Request` |
| `search` | `search` | no | `retrieval::search::Request` |
| `search_code` | `search-code` | no | `retrieval::search_code::Request` |
| `context` | `context` | no | `retrieval::context::Request` |
| `show` | `show` | no | `memories::show::Request` |
| `list` | `list` | no | `memories::list::Request` |
| `edges` | `edges` | no | `graph::edges::Request` |
| `repos` | `repos` | no | `code::repos::Request` |
| `recall_status` | `recall-status` | no | `learning::recall_status::Request` |
| `save` | `save` | yes | `memories::save::Request` |
| `feedback` | `feedback` | yes | `mcp::params::FeedbackParams` |

Every `repo` parameter left unset resolves to the server's default scope,
except on `repos`, which stays unscoped like `GET /api/v1/repos` because it
is the discovery surface that tells an agent which labels exist. A `save`
whose `repo` is empty after that resolution is refused (`repo_required`);
reads run unscoped when the session has no default, as `serve` does.

```rust
/// `feedback` tool parameters. Differs from `learning::feedback::Request`
/// in one rule: provenance defaults to implicit.
#[derive(Deserialize, JsonSchema)]
pub struct FeedbackParams {
    pub query_id: String,
    #[serde(default)] pub used: Vec<String>,
    #[serde(default)] pub irrelevant: Vec<String>,
    #[serde(default)] pub used_code: Vec<String>,
    #[serde(default)] pub irrelevant_code: Vec<String>,
    /// True only when the user stated the verdict. Stored as `manual`;
    /// everything else is `implicit` and never enters the golden harvest.
    #[serde(default)] pub confirmed_by_user: bool,
}
```

### Error classification (`src/utilities/error_code.rs`)

```rust
/// Transport-neutral class of a crate error. `serve::envelope` maps it to
/// an HTTP status; `mcp::result` maps everything but `Internal` to a
/// tool-level error.
pub enum Class { NotFound, Forbidden, BadRequest, Unprocessable, Conflict,
                 Unavailable, Locked, NotImplemented, Internal }

/// The `(code, class)` pair for every `Error` variant — an exhaustive
/// match, so adding a variant fails to compile rather than falling through.
pub fn classify(e: &Error) -> (&'static str, Class);
```

The code words are the ones `serve::envelope::status_and_code` emits today
(`not_found`, `forbidden`, `bad_request`, `confirmation_required`, `usage`,
`config`, `frontmatter`, `document`, `ast`, `json`, `vec_dim_mismatch`,
`unavailable`, `embedder_unavailable`, `index_running`, `id_collision`,
`cancelled`, `unsupported`, `schema_mismatch`, `store_locked` for a locked
SQLite via `store::busy::is_locked`, `not_found` for an `Io` of kind
`NotFound`, and `internal` for the rest). `mcp` adds two of its own, raised
before any core runs: `read_only` and `repo_required`, both `BadRequest`.

### Tool results

Success: `CallToolResult` whose `structured_content` is the same JSON object
`serve` returns as the envelope's `data` for that command, and one `text`
block carrying the same object serialized (MCP backward compatibility).
`find` therefore returns `{ "hits": [UnifiedHit…], "query_id": "q-…",
"meta": PageMeta }`; `save` returns the saved id and path; `feedback` returns
the recorded counts.

`get_info()` returns `ServerInfo { capabilities: tools, instructions }` where
`instructions` is the five-line loop: recall with `find` before exploring;
judge every hit you act on and call `feedback`; `save` verified corrections,
decisions and fixes with evidence, `supersedes` for outdated ones; `show` an
id before citing it; on a `store_locked` error wait a moment and retry the
same call once.

### `recall-status` output

```jsonc
{
  "repo": "comemory",
  "since": "2026-09-18T14:02:11Z",
  "queries": 4,            // tracked find/search/context/search-code rows
  "feedback_events": 1,    // verdicts recorded in the window
  "saves": 0,              // memories created in the window for this repo
  "pending": [             // tracked queries with no verdict yet
    { "query_id": "q-20260918-ab12cd34", "query": "why is X", "at": "…",
      "source": "find", "returned_ids": ["490ef5f7", "cad62a9b"] }
  ]
}
```

### `.mcp.json` (written by `install`)

```json
{
  "mcpServers": {
    "comemory": {
      "command": "/opt/homebrew/bin/comemory",
      "args": ["mcp"]
    }
  }
}
```

Claude Code names the tools `mcp__plugin_comemory_comemory__<tool>`.

### Hooks and config

`hooks.json` additions:

```json
"SessionEnd": [ { "hooks": [ { "type": "command",
  "command": "\"${CLAUDE_PLUGIN_ROOT}/hooks/session-end.sh\"" } ] } ]
```

`comemory.json` (user or project, project wins) gains one section:

```json
{ "recall": { "inject": true, "injectK": 3, "injectMinChars": 24,
              "enforce": true } }
```

Stop emits at most one advisory `systemMessage`, describing unjudged activity
in the shared repository window. It never emits `decision: "block"`; another
agent may own those queries. `recall-status` retains empty-id rows for accurate
telemetry, but the reminder skips them.

## Failure modes and edge cases

| Input or state | Behavior |
| --- | --- |
| No git repo at cwd and no `--repo` | Scope `None`. Reads run unscoped. `save` without `repo` → tool-level `repo_required`. `instructions` names the missing scope. |
| Store cannot be opened at spawn | Exit non-zero with the store error on stderr; the host shows the server failed. No retry loop. |
| `--read-only` and a mutating tool | Tool-level `read_only`, no write attempted, no permit taken; it outranks `repo_required` on an unscoped `save`. Tracked reads write no `retrieval_log` row. |
| Any crate `Error` whose `classify` class is not `Internal` (usage, not found, forbidden, bad request, vec-dim mismatch, id collision, index running, schema too new, locked store, …) | Tool-level `structured_error { code, message }` with the envelope's code word; a locked store surfaces as `store_locked`, which the instructions tell the agent to retry. |
| `Error` of class `Internal` (a non-lock `Sqlite`, a non-NotFound `Io`, `Git`, `Migration`, `Other`, …) | Protocol `ErrorData::internal_error` carrying only the code word; the message is logged to stderr, not echoed. |
| Concurrent tool calls | The connection mutex serializes them; it is never held across an `.await`, so no deadlock. |
| stdin EOF or SIGTERM | Clean exit `0`; nothing is flushed to the store beyond the completed call. |
| `RUST_LOG` set by the host | Every log line goes to stderr (`main.rs` installs the subscriber with `with_writer(stderr)`); stdout stays a byte-exact JSON-RPC stream. |
| `find` param `vector` supplied with the wrong dimension | Class `Unprocessable`: tool-level `vec_dim_mismatch`. |
| `install` re-run after the binary moved | `.mcp.json` is rewritten with the new path; the bundle equality check ignores it. |
| `install --dry-run` | Reports the `.mcp.json` path, writes nothing. |
| Injection: `comemory` absent, prompt under the floor, or in the skip list | No output. |
| Injection: `comemory` present but `find` fails, exceeds 5 s, or returns no hit | The plain reminder text, as before this change. `retrieval_log` untouched in every injection case (the hint runs with `COMEMORY_DISABLE_ACCESS_TRACKING=true`). |
| Injection output over 10,000 characters | Cannot happen: at most `injectK` lines of id and title. The script still truncates defensively. |
| Stop: no `session_id`, `stop_hook_active` true, `recall-status` error, already advised once, `recall.enforce` false | No advisory. |
| Stop: recalls but a save happened | No advisory; the window already includes a save. |
| SessionEnd: not logged in or platform down | Detached child fails quietly; the hook exits `0` in under 1.5 s. |
| Parity: a catalog entry names a subcommand clap does not have, or its parameter type rejects one of that subcommand's arg ids | `tests/mcp__parity.rs` fails naming the tool and the arg. |

## Acceptance criteria

- **AC-1:** Given `comemory mcp` spawned over stdio under a throwaway data
  dir, an rmcp client's `initialize` succeeds and `tools/list` returns exactly
  the eleven catalog names, each with a non-empty description and an
  `inputSchema` object.
- **AC-2:** Given two saved memories ("Use Postgres for analytics" and "Prefer
  nextest"), `tools/call find { "query": "postgres analytics" }` returns
  structured content whose first hit has the Postgres memory's id, plus a
  `query_id` that `retrieval_log` contains.
- **AC-3:** Given the server spawned with cwd inside a linked worktree of a
  repo whose main worktree is `sample`, `save` without `repo` files the memory
  under `sample`, and a second server spawned in the main worktree finds it
  with `find`.
- **AC-4:** Given `comemory mcp --read-only`, `save` and `feedback` return a
  tool-level `read_only` error, `find` still returns hits, and the
  `retrieval_log` row count is unchanged afterwards.
- **AC-5:** Given a tracked `find`, `feedback { used: [id] }` stores a
  `feedback_events` row with provenance `implicit`; the same call with
  `confirmed_by_user: true` stores `manual`.
- **AC-6:** `tests/mcp__parity.rs` walks `Cli::command()`: every catalog
  entry's command exists, `mcp` reports `transport: "cli-only"` with no
  routes and `recall-status` reports `transport: "http"` in a live `GET
  /api/v1/commands`, and every clap arg id of each catalogued subcommand
  deserializes into that tool's parameter type without an `unknown field`
  error. The exclusion set is exactly the rows of `tests/api__parity.rs`
  `EXCLUSIONS` that touch the catalog: `("save" | "find" | "search" |
  "search-code" | "context", "vector_stdin")` plus `("search", "only")` and
  `("search", "path")`, whose fields live in `cli::search_only` and never
  reach `retrieval::search::Request`; `recall-status` has none.
- **AC-7:** `comemory install claude --config-dir <tmp>` and `install codex`
  each write `<bundle>/plugins/comemory/.mcp.json` whose `command` equals the
  test binary's absolute path and `args` equals `["mcp"]`; `--dry-run` leaves
  no such file. The file is written under the temp config dir only:
  `guardrails.config.json` lists `.mcp.json` under `secrets.neverTracked`, so
  no fixture may commit one.
- **AC-8:** Given one tracked `find` and no verdict, `comemory recall-status
  --repo R --json` reports `pending` of length 1 naming that query id; after
  `comemory feedback <id> --used <mid>` it reports 0 pending and 1
  `feedback_events`; `GET /api/v1/learning/recall-status` returns the same
  object.
- **AC-9:** Given a saved memory titled "Rebase on main before push" and the
  real `memory-lifecycle.sh` fed a `UserPromptSubmit` payload with prompt
  "how do I push this branch safely", the hook's `additionalContext` contains
  that memory's id and title; with prompt "ok" it emits nothing; with the
  same prompt but `comemory` absent from `PATH` it emits nothing and exits
  `0`; the `retrieval_log` row count is unchanged in every case.
- **AC-10:** A repository window with an unjudged recall containing ids can
  emit one advisory `systemMessage`; a repeated Stop is silent. Two overlapping
  sessions never block each other. Empty-id recalls, a recorded verdict/save,
  and `stop_hook_active` produce no reminder.
- **AC-11:** `SessionEnd` returns quietly without waiting for upload. Claude
  uses the supported transcript parser; Codex exits without attempting capture.
- **AC-12:** `comemory-status.sh` on a repo with zero memories emits
  `additionalContext` naming `memory-bootstrap`; with one memory saved it
  emits none.
- **AC-13:** `agent-memory/SKILL.md` names every catalog tool and the wrapper
  fallback; `memory-bootstrap/SKILL.md` exists with `## When to Use`,
  `## Procedure`, `## Pitfalls`, `## Verification`; `scripts/test-agent-install.sh`
  and `scripts/test-project-skills.sh` pass.
- **AC-14:** `bash scripts/check-all.sh` passes: architecture check and
  inventory with `mcp` staged and every new file inventoried, guardrails
  with `mcp` in `src.topLevel` and `src/mcp` in `requireReadme` (no file over
  300 code lines), `cli-docs-check`, the scenario catalog with
  `docs/scenarios/mcp.md` and `docs/scenarios/recall-status.md`, dup-check,
  and `store-leak-baseline.txt` still `0`. Seven flip-and-revert proofs, one
  per gate item in § Architecture, are recorded in the PR description with
  the failing line each produced: (1) `mcp` removed from the policy arrays →
  `invalid policy or inventory metadata`; (2) one new inventory row deleted →
  `inventory coverage mismatch`; (3) a `use crate::mcp;` line added to a
  domain file → `delivery dependency`; (4) `bash
  scripts/test-architecture-policy.sh` passes with its new
  `delivery::cli`-on-`src/mcp/` case; (5) a `src/mcp/` row's owner flipped
  to `delivery::cli` → `capability ownership mismatch`; (6) `mcp` removed
  from `guardrails.config.json` `src.topLevel` → guardrails fail, and
  `src/mcp/README.md` deleted → guardrails fail; (7) `mcp` removed from the
  `tests/api__parity.rs` mirror → that test fails with `no registered route`.
- **AC-15:** `cargo deny check` passes with `rmcp` and `schemars` added.

## Acceptance evidence

| AC | Real input / fixture | Expected observable | Boundary case | Runnable check |
| --- | --- | --- | --- | --- |
| AC-1 | `tests/common/mcp_bin.rs` spawns `cargo_bin("comemory") mcp` under `<tmp>/.comemory`; rmcp client over child stdio | `tools/list` names == catalog | `--read-only` lists the same eleven | `tests/cli_scenario_mcp.rs::mcp_01_lists_catalog` |
| AC-2 | two `comemory save` calls via `cli_save_support` | first hit id, `query_id` present in `retrieval_log` | empty corpus → zero hits, still a `query_id` | `tests/cli_scenario_mcp.rs::mcp_02_find_returns_hits` |
| AC-3 | `tests/common/git_worktree.rs` builds `sample` + linked worktree | save from worktree visible from main | no git repo → `repo_required` on save | `tests/cli_scenario_mcp.rs::mcp_03_scope_is_main_worktree` |
| AC-4 | same server with `--read-only` | `read_only` code on `save`/`feedback`, `find` works | log count equal before and after | `tests/cli_scenario_mcp.rs::mcp_04_read_only` |
| AC-5 | tracked `find` then `feedback` twice | provenance `implicit` then `manual` via `store::feedback::event_counts`-style read | unknown `query_id` → `not_found` | `tests/cli_scenario_mcp.rs::mcp_05_feedback_provenance` |
| AC-6 | `Cli::command()` and a live `comemory serve` | zero unmapped arg ids; transports `cli-only` for `mcp`, `http` for `recall-status` | a deliberate extra field fails the probe | `tests/mcp__parity.rs` |
| AC-7 | `install <host> --config-dir <tmp> --json` per host | `.mcp.json` path and `command` | `--dry-run` → no file | `tests/cli_scenario_install.rs::install_writes_mcp_manifest`, `scripts/test-agent-install.sh` |
| AC-8 | one tracked `find` via CLI, then `feedback` | pending 1 → 0 | `--since` in the future → all zeros | `tests/cli__recall_status.rs`, `tests/serve__routes__learning.rs::recall_status_route` |
| AC-9 | real hook script, real binary, saved memory, JSON payload on stdin | id and title in `additionalContext`; nothing for "ok" | `comemory` absent from `PATH` → nothing, exit 0 | `scripts/test-agent-install.sh` (new block) |
| AC-10 | real hook script with a session start marker and a tracked `find` | advisory once, then silence; never block another agent | `stop_hook_active: true` → silence | `scripts/test-agent-install.sh` (new block) |
| AC-11 | real `session-end.sh`, real payload, unauthenticated binary | exit 0, empty stdout, elapsed < 1.5 s | `comemory` absent from PATH → exit 0 | `scripts/test-agent-install.sh` (new block) |
| AC-12 | real `comemory-status.sh` with 0 then 1 memory | nudge then none | count command fails → no output | `scripts/test-agent-install.sh` (new block) |
| AC-13 | the SKILL.md files | tool names present; four sections present | — | `scripts/test-project-skills.sh`, `scripts/test-agent-install.sh` |
| AC-14 | the tree | gate green | the seven flip-and-revert proofs named in AC-14 each fail loudly (memory `gates-can-pass-while-checking-nothing`) | `bash scripts/check-all.sh`, `bash scripts/test-architecture-policy.sh`, `bash scripts/test-architecture-check.sh` |
| AC-15 | `Cargo.lock` | deny green | — | `bash scripts/deny-check.sh` |

## Documentation impact

- `README.md`: agent hosts section names `comemory mcp` and the `.mcp.json`
  registration.
- `AGENTS.md`: replace "not a Claude Code MCP plugin" with the adapter
  stance; Module Map row for `mcp/`; Key Commands `comemory mcp`,
  `comemory recall-status`; Environment Variables unchanged.
- `docs/architecture.md`: delivery adapters are now three.
- `docs/guides/agent-integration.md`: MCP registration per host, manual
  registration for Cursor / Gemini CLI / Windsurf, the `recall` config keys,
  the advisory behavior and how to turn it off, SessionEnd capture.
- `docs/guides/ranking-and-eval.md`: agent verdicts are `implicit` by default.
- `docs/cli-reference.md`: regenerated (`scripts/regen-cli-docs.sh`).
- `docs/scenarios/mcp.md`, `docs/scenarios/recall-status.md`, journey rows in
  `docs/scenarios/README.md`; `docs/scenarios/install.md` gains the manifest
  scenario; `docs/scenarios/feedback.md` unchanged.
- `docs/README.md`: designs index entry for this document.
- `docs/guides/runtime-orm.md`: row for the hand-SQL
  `store::retrieval_log::pending_since`.
- `docs/designs/2026-09-17-domain-first-migration-inventory.md`: one row per
  new production file (§ Architecture item 5).
- `src/mcp/README.md` (new), rows in `src/cli/README.md`,
  `src/config/README.md`, `src/domains/learning/README.md`,
  `src/store/README.md`, `src/utilities/README.md`, `src/serve/README.md`
  (the `run_blocking` and `status_and_code` moves).
- Gate configuration, not docs but user-visible in review:
  `guardrails.config.json`, `scripts/architecture-policy.json`,
  `scripts/lib/architecture-inventory.sh`, `scripts/architecture-check.sh`,
  `scripts/test-architecture-policy.sh`, `src/serve/routes/meta.rs`
  `CLI_ONLY`, `tests/api__parity.rs` `CLI_ONLY` mirror.
- `integrations/README.md`, `integrations/agent/skills/agent-memory/SKILL.md`,
  `integrations/agent/skills/memory-bootstrap/SKILL.md` (new).
- Release notes come from the conventional commit via release-plz.

## Open Questions

1. **Stop attribution.** Review correction: the repository/time window does
   not identify an agent session. Reminders are advisory on both hosts; true
   per-session enforcement would require explicit attribution and is deferred.
2. **Router composition across two files.** Resolved 2026-09-18: rmcp
   documents `#[tool_router(router = name, vis = "pub")]` per impl block and
   `ToolRouter<S>` implements `Add`, so `server.rs` builds
   `read_router() + write_router()`.
3. **`schemars` derive on domain `Request` types.** Resolved 2026-09-18:
   `scripts/architecture-check.sh` line 314 keeps only `crate::`-rooted
   paths, so an external derive is invisible to the layer check. The derive
   goes on the domain types; there is no newtype fallback.
4. **Expose `vector` on MCP `find`/`search`/`save`?** Kept, because the
   domain `Request` carries it and hiding it would need a second type.
   Non-blocking; revisit if hosts choke on the array schema.

## Review corrections and additional acceptance (2026-09-19)

- `context` returns full bodies and linked data; it has no token budget.
  Recommend `find(k=3)` then selected `show` calls. `edges` is lexical triplet
  search, not adjacency lookup by id. `find` includes documents.
- MCP opens/drops one connection per call under its session mutex. A rebuild
  between calls must be visible to every agent (`mcp_07` journey).
- Concurrent fresh opens apply each migration once. Concurrent MCP saves,
  including identical-body replays, must preserve the markdown/index contract
  (`simultaneous_first_opens_apply_each_migration_once`, `mcp_08`).
- Mixed memory/code feedback must commit in one immediate transaction; a
  code identity error must roll back memory verdicts too. Concurrent verdicts
  must not fail from a deferred read-to-write upgrade.
- Native Claude and Codex installers must expose the MCP server in each host's
  `mcp list`, not merely leave a manifest on disk. Claude's health check must
  complete the connection. Generic stdio clients exercise protocol negotiation.
- Disabled integration emits no bootstrap hint. Capture is Claude-only, and
  bootstrap must not pass unsupported `--repo` to `distill`.
