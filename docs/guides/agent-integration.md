# Agent skills and hooks

Comemory owns its Claude Code and Codex integration. Install both the skills and
lifecycle hooks with one command:

```sh
comemory install claude
comemory install codex
```

`claude-hooks` and `codex-hooks` are accepted aliases. `--dry-run` previews the
bundle location without modifying anything. `--json` returns the installation
report. `--config-dir DIR` targets an isolated host configuration directory;
otherwise the host honors `CLAUDE_CONFIG_DIR` / `CODEX_HOME` and its usual default.

The corresponding host CLI, Bash, Git, jq, and Python 3 must be available. The
binary embeds the integration and extracts a local marketplace beneath
`$COMEMORY_DATA_DIR/integrations/<version>` (default `~/.comemory`). It registers
`comemory@comemory` through the host's native plugin manager. The marketplace
path stays at `integrations/` across releases so native registration remains stable. No source checkout
or toolu installation is required. Re-running an identical version is safe;
modified bundle files are preserved and reported as a conflict. Restart the
host after installation. After upgrading comemory, rerun the install command.

SessionStart publishes `comemory/comemory.sh` and `comemory/skills.sh` under the
host configuration directory. The memory wrapper uses canonical Git scope so
worktrees share knowledge; `--repo NAME` selects an explicit scope. The agent
recalls targeted knowledge, verifies its work, compares existing memories,
saves useful corrections and discoveries, and records retrieval feedback.

Prompt and compaction hooks provide compact reminders. Stop runs local retrieval
maintenance at most once per UTC day in a detached process. These hooks do not
invoke models or upload transcripts.

## MCP registration

`comemory install claude` and `comemory install codex` also write
`<bundle>/plugins/comemory/.mcp.json` — the plugin root both Claude Code and
Codex read to spawn `comemory mcp` — on every install, with the installing
binary's absolute path:

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

`--dry-run` reports the path and writes nothing. Re-running `install` after
the binary moved rewrites the path; the bundle's embedded-file equality check
ignores this file.

Claude Code names the registered tools `mcp__plugin_comemory_comemory__<tool>`;
other MCP hosts list them by their bare name. Eleven tools, reads first:

| Tool | Purpose |
| --- | --- |
| `find` | Recall memories and code for a natural-language question — ranked hits across both corpora plus a `query_id`. |
| `search` | Search saved memories only — ranked hits with ids, titles and scores plus a `query_id`. |
| `search_code` | Search indexed code symbols only — ranked functions, types and methods plus a `query_id`. |
| `context` | Assemble a token-budgeted briefing for a task — memories and code symbols, already ordered. |
| `show` | Fetch one memory by its 8-hex id — full body, frontmatter and relations. |
| `list` | Page through live memories with optional repo, kind, tag and quality filters. |
| `edges` | List the graph edges touching a node, with each neighbour's relation and direction. |
| `repos` | List the indexed repositories with their symbol and memory counts. |
| `recall_status` | Report the recall loop's state for a repo since a timestamp — query counts, verdicts, saves, and pending `query_id`s. |
| `save` | Store a memory — returns the new 8-hex id and file path. |
| `feedback` | Record which recalled ids you actually used — returns the counts stored against that `query_id`. |

Scope follows the same rule everywhere in comemory: a repo is the main
worktree's basename (`domains::code::git_utils::repo_label_at`); a linked
worktree is never a repository of its own. Every `repo` parameter left unset
resolves to the server's default scope (`--repo`, else the cwd's main
worktree), except `repos`, which stays unscoped — like `GET /api/v1/repos` —
because it is the discovery surface that tells an agent which labels exist. A
`save` whose `repo` is still empty after that resolution is refused with a
tool-level `repo_required` error before anything is written.

`comemory mcp --read-only` refuses `save` and `feedback` with a tool-level
`read_only` error; the nine read tools still work, and a read run under
`--read-only` writes no `retrieval_log` row.

### Manual registration (Cursor, Gemini CLI, Windsurf, non-plugin Codex)

Comemory ships no plugin bundle for these hosts; register `comemory mcp` by
hand as a generic stdio MCP server:

```json
{ "command": "comemory", "args": ["mcp"] }
```

For Codex outside the plugin:

```bash
codex mcp add comemory -- comemory mcp
```

`--repo NAME` and `--read-only` work the
same way regardless of how the host launched the process.

## Recall injection and Stop enforcement

**Injection** (`UserPromptSubmit`, `memory-lifecycle.sh`) runs `comemory find`
scoped to the repo, memory domain, `k` from `recall.injectK`, with
`COMEMORY_DISABLE_ACCESS_TRACKING=true` — the hint mints no `retrieval_log`
row; the agent's own `find` call is the tracked one. It is skipped for
prompts under `recall.injectMinChars`, for a short skip list (`ok`, `thanks`,
`/comemory:*`, …), and when the call exceeds five seconds or `comemory` is not
on `PATH`. Any of those falls back to the plain standing reminder. Turn it off
with `"recall": { "inject": false }`.

**Enforcement** (`Stop`, `memory-lifecycle.sh`) calls `comemory recall-status
--repo KEY --since <session start>`. When the session has tracked recalls and
recorded neither a `feedback` verdict nor a `save`, the hook returns
`{"decision":"block","reason":…}` naming the pending `query_id`s:

```json
{ "decision": "block",
  "reason": "comemory: 2 recalls this session have no verdict and nothing was saved. Call feedback (used/irrelevant) for q-…, q-…, or save the lesson, then stop." }
```

It fires **at most once per session** — a `session-<id>.blocked` marker keeps
a second `Stop` silent — and it **never blocks when `stop_hook_active` is
true** (both hosts send that flag on a turn that is already a continuation;
Claude Code additionally caps consecutive blocks at eight on its own side).
It never blocks on error, on a missing session-start marker, or when a save
happened even without a verdict. Turn it off with `"recall": { "enforce":
false }`, or query what is still pending yourself with `comemory
recall-status`.

## SessionEnd capture

The bundled plugin's `hooks.json` registers a `SessionEnd` hook,
`hooks/session-end.sh`, which launches `comemory capture session --from-hook`
**detached** and exits within the host's ~1.5s `SessionEnd` budget regardless
of whether the platform round trip finishes. Capture requires `comemory auth
login`; without it (or with capture consent withheld) the detached call fails
quietly and the hook still exits `0`. `comemory install claude` and `comemory
install codex` cover this automatically — no separate step. Non-plugin
setups keep `comemory capture install-hook`, which installs the same
`SessionEnd` → `capture session --from-hook` command directly into the host's
own settings file.

The project-skills workflow preserves `.toolu/skills/` as its storage directory
for compatibility with existing procedures. It runs independently of toolu.
Configure it in the host's `comemory.json` or the repository's
`.claude/comemory.json` / `.codex/comemory.json` (project values override user
values):

```json
{
  "skills": { "comemory": true },
  "projectSkills": {
    "enabled": true,
    "staleAfterDays": 30,
    "archiveAfterDays": 90,
    "indexCap": 20
  },
  "recall": { "inject": true, "injectK": 3, "injectMinChars": 24, "enforce": true }
}
```

Set `skills.comemory` to `false` to suppress integration context and project-skill
hooks, or disable the plugin in the host. Set `projectSkills.enabled` to `false`
to disable only the project-skill loop. The optional memory-count marker remains
compatible with toolu's statusline.

The `recall` section controls the prompt-time recall injection and Stop-time
enforcement described above, all defaulting on:

| Key | Default | Effect |
| --- | --- | --- |
| `recall.inject` | `true` | On `UserPromptSubmit`, run an untracked `find` over the prompt and inject the top hits' ids and titles as `additionalContext`, instead of the plain reminder text. |
| `recall.injectK` | `3` | How many hits the injected hint carries. |
| `recall.injectMinChars` | `24` | Prompts shorter than this get no injection attempt — too little text to search on. |
| `recall.enforce` | `true` | On `Stop`, block once per session when the session made tracked recalls with neither a verdict nor a save. |

## Migrating from toolu

First install `comemory@comemory`, then disable or uninstall `comemory@toolu`
using the host's plugin manager. Keep only one enabled to avoid duplicate hooks.
Memory files, the database, and `.toolu/skills/` do not move. Copy any wanted
`projectSkills` settings from `toolu.config.json` into `comemory.json`.

This installer registers the `SessionEnd` capture hook (above) and MCP
registration as part of the plugin bundle, but it does not itself upload
sessions — that still needs `comemory auth login` and platform capture
consent — and it does not install Git indexing hooks. Git hooks remain
available through `comemory install-hooks`. Learning checkpoints are a
separate follow-up to this ownership migration.

### Upgrading the bundle

Re-running `comemory install claude` / `comemory install codex` over an
existing bundle (not just a fresh install) picks up two behavior changes on
the next host restart: `Stop` can now block once per session when this
session made tracked recalls with neither a `feedback` verdict nor a `save`
(see [Recall injection and Stop enforcement](#recall-injection-and-stop-enforcement)
above), and prompt-time injection replaces the old standing reminder text on
`UserPromptSubmit` with the top matching memories themselves. Turn either off
independently with `"recall": { "enforce": false }` or `"recall": { "inject":
false }` in `comemory.json`.
