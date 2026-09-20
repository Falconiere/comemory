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
maintenance at most once per UTC day in a detached process. Recall and maintenance
run locally without models. The separate Claude SessionEnd receipt capture is
described below.

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
| `find` | Search memories, code and documents — ranked hits plus a `query_id`. Start with `k: 3`. |
| `search` | Search saved memories only — ranked hits with ids, titles and scores plus a `query_id`. |
| `search_code` | Search indexed code symbols only — ranked functions, types and methods plus a `query_id`. |
| `context` | Return full memory bodies and linked code/relations. `k` limits memory count, not tokens. |
| `show` | Fetch one memory by its 8-hex id — full body, frontmatter and relations. |
| `list` | Page through live memories with optional repo, kind, tag and quality filters. |
| `edges` | Search relation triplets by title/path/relation words; this is not an adjacency lookup by node id. |
| `repos` | List the indexed repositories with their symbol and memory counts. |
| `recall_status` | Report shared repository activity since a timestamp; this is not a per-agent or per-session report. |
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

## Efficient recall across agents

Start with `find {"query":"specific task", "k":3}`. Read only the useful
memory ids with `show`; read code/document hits at their paths. A prompt hint
already supplies candidate memory ids, so use selected `show` calls without
repeating the same search. These untracked hints have no `query_id`; `show`
alone needs no feedback. Request more hits only when those results are
insufficient. Record feedback for tracked results actually used, and save only
verified reusable knowledge; an empty recall needs neither an invented verdict
nor a memory just to satisfy a hook.

`context` includes complete memory bodies and linked content. It has no token
budget, even with `k: 1`; use it only when that larger bundle is useful. Tool
results carry both `structuredContent` and a JSON text block for older MCP
clients, following the [MCP compatibility guidance](https://modelcontextprotocol.io/specification/2025-06-18/server/tools#structured-content).
Hosts decide which representation reaches the model; wire size is not a
model-token count.

Each host can run its own stdio server over the same data directory. Database
setup is serialized across processes, saves reserve SQLite's writer before
reading rows to update, and each MCP call opens the current database. A rebuild
between calls therefore does not leave idle agents on the replaced file. Run
rebuild during a quiet period; it is not an online operation coordinated with
active writes. On `store_locked`, retry the same call once after a short wait.
Feedback commits memory and code verdicts together in one immediate transaction,
so a failed mixed request cannot leave only its memory counters incremented.

A server's default repo is fixed at launch. An agent working on a different
repository must pass `repo` explicitly; changing a shell's cwd does not change
an existing MCP server's scope. `COMEMORY_DATA_DIR` and other environment
settings must also be present in the host/server environment, not only in the
shell that ran `install`.

## Recall injection and Stop reminders

**Injection** (`UserPromptSubmit`) runs an untracked `find` in the memory
domain with `recall.injectK` hits. It injects ids and titles, capped at 4,000
characters. The hook attempts recall only above `recall.injectMinChars`; a
short skip list (`ok`, `thanks`, `/comemory:*`, …) and a missing binary produce
no output. Other misses/errors fall back to a compact MCP reminder. When
`timeout` or `gtimeout` is installed, lookup is bounded to five seconds; without
either, the external command has no hook-local timeout. Disable lookup with
`"recall": {"inject": false}`.

**Stop reminders** retain the `recall.enforce` setting for compatibility, but
are advisory: they never return `decision: "block"`. `recall-status --repo
KEY --since <session start>` reports repository-wide activity, so overlapping
agents can contribute queries, verdicts and saves to the same window. A
reminder must not claim those actions belong to one session or require another
agent to judge them.

The hook reminds at most once per session, skips `stop_hook_active`, and stays
silent on missing markers or command errors. It omits pending rows with no
recorded ids from the reminder. Those rows remain visible in `recall-status`;
`find` currently logs memory ids only, so an empty logged id list can also mean
a code/document-only result. Judge actual results you used, rather than
inventing ids for a status row. Disable reminders with
`"recall": {"enforce": false}`.

## SessionEnd capture

The shared plugin registers `hooks/session-end.sh`, but receipt capture runs
only under **Claude Code**: the current capture/distill parser accepts Claude
transcripts. Under Codex this hook exits silently; MCP tools, memory skills and
local recall still work. Codex transcript capture is not implemented.

For Claude, the hook launches `comemory capture session --from-hook` detached
and exits without waiting for the platform. Capture needs `comemory auth login`
and platform capture consent; unavailable capture fails quietly. Non-plugin
Claude setups can use `comemory capture install-hook`. Receipt capture uploads
metadata/redaction information, not the raw transcript; see
[session capture](session-capture.md).

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
reminders described above, all defaulting on:

| Key | Default | Effect |
| --- | --- | --- |
| `recall.inject` | `true` | On `UserPromptSubmit`, run an untracked `find` over the prompt and inject the top hits' ids and titles as `additionalContext`. |
| `recall.injectK` | `3` | How many hits the injected hint carries. |
| `recall.injectMinChars` | `24` | Prompts shorter than this get no injection attempt — too little text to search on. |
| `recall.enforce` | `true` | On `Stop`, emit at most one advisory about shared repo activity; never block an agent. |

## Migrating from toolu

First install `comemory@comemory`, then disable or uninstall `comemory@toolu`
using the host's plugin manager. Keep only one enabled to avoid duplicate hooks.
Memory files, the database, and `.toolu/skills/` do not move. Copy any wanted
`projectSkills` settings from `toolu.config.json` into `comemory.json`.

This installer registers MCP and the shared lifecycle hooks, including
Claude-only SessionEnd capture (above), but does not itself upload sessions — that still needs `comemory auth login` and platform capture
consent — and it does not install Git indexing hooks. Git hooks remain
available through `comemory install-hooks`. Learning checkpoints are a
separate follow-up to this ownership migration.

### Upgrading the bundle

After upgrading the binary, rerun the host's install command and restart the
host so its cached plugin picks up the new bundle. Reinstalling the identical
version remains safe; changed embedded files under the same version are a
conflict and are not overwritten silently. Prompt hints prefer selected `show`
calls, and Stop reminders are advisory for concurrent-agent safety. The two
settings `recall.inject` and `recall.enforce` disable each independently.
