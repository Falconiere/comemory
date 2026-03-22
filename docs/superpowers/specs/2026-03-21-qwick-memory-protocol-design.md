# qwick-memory: Automatic Memory Protocol for qwick-memory

**Date:** 2026-03-21
**Status:** Draft

## Goal

Add an aggressive, always-on memory protocol to qwick-memory that automatically captures decisions, bugs, conventions, discoveries, and session context — replicating engram's behavior using qwick-memory's infrastructure (markdown files, vector search, git-shareable).

Once active, qwick-memory replaces engram. Disable the engram plugin when qwick-memory is enabled to avoid conflicting protocols.

## Overview

The system has two parts:

1. **Protocol (instructions)** — Injected via MCP server `instructions` parameter, tells Claude when and how to call qwick-memory tools proactively. This is the primary mechanism — Claude's judgment decides what to save during conversation.
2. **Lifecycle hooks** — Handle session start (context loading), pre-compaction (best-effort reminder to save summary), and post-compaction (restore context).

## Tool Renaming

All existing MCP tools are renamed from `rag_*` to `qwick_memory_*`:

| Old Name | New Name |
|----------|----------|
| `rag_save` | `qwick_memory_save` |
| `rag_search` | `qwick_memory_search` |
| `rag_list` | `qwick_memory_list` |
| `rag_delete` | `qwick_memory_delete` |
| `rag_index` | `qwick_memory_index` |
| `rag_context` | `qwick_memory_context` |
| *(new)* | `qwick_memory_session_summary` |

The CLI commands (`qwick-memory save`, `qwick-memory search`, etc.) remain unchanged — only MCP tool names change.

## Memory Type Addition

Add `"session-summary"` to `MEMORY_TYPES` in `memory.py`. This is the one change to `memory.py` — it enables clean filtering of session summaries vs. durable knowledge.

## MCP Server Protocol

The FastMCP server gets an `instructions` parameter with the memory protocol. This is injected into Claude's context automatically when the MCP server connects.

### Protocol Content

```
## Qwick Memory — ACTIVE PROTOCOL

You have qwick-memory tools (qwick_memory_save, qwick_memory_search, qwick_memory_context, qwick_memory_session_summary).
This protocol is MANDATORY and ALWAYS ACTIVE.

### PROACTIVE SAVE — do NOT wait for user to ask
Call `qwick_memory_save` IMMEDIATELY after ANY of these:
- Decision made (architecture, convention, workflow, tool choice)
- Bug fixed (include root cause)
- Convention or workflow documented/updated
- Non-obvious discovery, gotcha, or edge case found
- Pattern established (naming, structure, approach)
- User preference or constraint learned
- Feature implemented with non-obvious approach
- Artifact created or updated with significant content (Notion, Jira, GitHub, etc.)

**Self-check after EVERY task**: "Did I just make a decision, fix a bug, learn something, or establish a convention? If yes → qwick_memory_save NOW."

When saving, choose the right type:
- `decision` — architecture, convention, workflow, tool choice
- `bug` — bug fixed, include root cause and fix
- `convention` — coding standard, workflow rule, naming pattern
- `discovery` — non-obvious finding, gotcha, edge case
- `pattern` — recurring approach, structure, design pattern
- `preference` — user preference, constraint, working style
- `note` — anything else worth remembering
- `session-summary` — (used automatically by qwick_memory_session_summary, do not use directly)

Use descriptive, comma-separated tags for discoverability.

### SEARCH MEMORY when:
- User asks to recall anything ("remember", "what did we do", "acordate", "que hicimos")
- Starting work on something that might have been done before
- User mentions a topic you have no context on
- User's FIRST message references the project, a feature, or a problem — call `qwick_memory_search` with keywords to check for prior work before responding

### SESSION CLOSE — before saying "done"/"listo":
Call `qwick_memory_session_summary` with a structured summary:
- Goal: what the user wanted to accomplish
- Discoveries: non-obvious things learned
- Accomplished: what was done
- Next steps: what remains
- Relevant files: key files touched or referenced
```

## New MCP Tool: `qwick_memory_session_summary`

### Signature

```python
async def qwick_memory_session_summary(
    goal: str,
    discoveries: str,
    accomplished: str,
    next_steps: str,
    relevant_files: str,
) -> str
```

### Behavior

1. Combines the 5 fields into a structured markdown body:

```markdown
## Session Summary

**Goal:** {goal}

**Discoveries:**
{discoveries}

**Accomplished:**
{accomplished}

**Next Steps:**
{next_steps}

**Relevant Files:**
{relevant_files}
```

2. Validates that `goal` is non-empty (return error if blank).

3. Saves as a memory with:
   - `type`: `"session-summary"`
   - `tags`: `["session-summary"]`
   - Standard atomic write flow (temp file → embed → upsert → rename)

4. **Rotation policy:** After saving, delete all but the 3 most recent session summaries for the current repo. This prevents accumulation over time.

5. Returns confirmation string.

## Enhanced `qwick_memory_context`

Context output is structured in two sections:

1. **Latest session summary** (if any) — the single most recent `session-summary` type memory for the repo, shown first under a "Last Session" header.
2. **Recent memories** — up to `limit` non-summary memories, sorted by created date descending.

This ensures context restoration after compaction includes both session state and durable knowledge without summaries crowding out useful memories.

## Hooks

### hooks/hooks.json

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PLUGIN_ROOT}/scripts/session-start.sh\"",
            "async": false
          }
        ]
      }
    ],
    "PreCompact": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PLUGIN_ROOT}/scripts/pre-compact.sh\"",
            "async": false
          }
        ]
      }
    ],
    "PostCompact": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PLUGIN_ROOT}/scripts/post-compact.sh\"",
            "async": false
          }
        ]
      }
    ]
  }
}
```

### scripts/session-start.sh (enhanced)

1. Auto-index (existing behavior)
2. Output recent context via `qwick-memory context` CLI — this text appears in Claude's context as the hook result

All scripts are **self-locating** — they resolve the project root from their own physical location via `dirname`, then use `uv run --directory` to find the package. This works from any working directory.

```bash
#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
UV="uv run --directory $PROJECT_ROOT"

# Auto-index
if [ -d "$PROJECT_ROOT/memories" ]; then
  $UV python -m qwick_memory index 2>/dev/null || true
fi

# Output context for Claude
echo "## Qwick Memory — Session Context"
echo ""
$UV python -m qwick_memory context 2>/dev/null || echo "No prior context found."
```

### scripts/pre-compact.sh (new)

A best-effort reminder before compaction. The shell hook outputs a message that Claude sees. However, compaction may proceed before Claude can act on it — this is a known limitation. The primary defense is the protocol's "SESSION CLOSE" instruction, which tells Claude to call `qwick_memory_session_summary` proactively before ending work. The PreCompact hook is a safety net, not the primary mechanism.

```bash
#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
UV="uv run --directory $PROJECT_ROOT"

echo "## Qwick Memory — Pre-Compaction Notice"
echo ""
echo "Context compaction is about to happen."
echo "If you haven't already, call qwick_memory_session_summary now."
echo ""
echo "Current memory state:"
$UV python -m qwick_memory context --limit 5 2>/dev/null || echo "No context available."
```

### scripts/post-compact.sh (new)

Restores context after compaction by outputting recent memories.

```bash
#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
UV="uv run --directory $PROJECT_ROOT"

echo "## Qwick Memory — Context Restored After Compaction"
echo ""
echo "Context was just compacted. Here are your recent memories:"
echo ""
$UV python -m qwick_memory context 2>/dev/null || echo "No prior context found."
```

## CLI Enhancement: `context` command

Add a `context` subcommand to the CLI (currently only exists as MCP tool). This allows hook scripts to call it directly.

```python
@app.command()
def context(
    repo: str | None = None,
    limit: int = typer.Option(10, help="Maximum number of memories to return."),
) -> None:
    """Show recent memories for context restoration."""
    # Same structured output as qwick_memory_context:
    # 1. Latest session summary (if any)
    # 2. Recent non-summary memories, sorted by created desc
    # Output to stdout as plain text for hook consumption
```

## Changes Summary

| File | Change |
|------|--------|
| `server.py` | Rename all tools `rag_*` → `qwick_memory_*`, add `instructions` to FastMCP, add `qwick_memory_session_summary` tool, enhance `qwick_memory_context` with structured output |
| `cli.py` | Add `context` subcommand with `--limit` option |
| `memory.py` | Add `"session-summary"` to `MEMORY_TYPES` |
| `hooks/hooks.json` | Add `PreCompact` and `PostCompact` hook entries |
| `scripts/session-start.sh` | Add context output after indexing |
| `scripts/pre-compact.sh` | New script — best-effort reminder + context snapshot |
| `scripts/post-compact.sh` | New script — restore context |
| `CLAUDE.md` | Update module map (7 tools), add protocol section |

**No changes to:** `index.py`, `search.py`, `config.py`, `errors.py`, `git_utils.py`

## Testing

- Existing tests for renamed tools: update tool function names in MCP server tests
- New test for `qwick_memory_session_summary`:
  - Verify saves structured summary with `type="session-summary"` and `tags=["session-summary"]`
  - Verify rotation: after saving 4+ summaries, only 3 most recent remain
  - Verify error on empty `goal`
- New test for `qwick_memory_context`:
  - Verify latest session summary shown first, then non-summary memories
  - Verify empty state (no memories)
  - Verify with session summaries present (ordering)
- New test for CLI `context` command: verify output format, `--limit` flag, empty state
- Hook scripts: e2e test updates (verify scripts exist and are executable)
- Protocol: manual verification — start a session, observe Claude proactively saving and searching
- e2e test: add `context` subcommand checks to `scripts/e2e-test.sh`

## Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| PreCompact hook timing — Claude may not act before compaction | Best-effort safety net; primary mechanism is protocol's proactive "SESSION CLOSE" instruction |
| Tool rename breaks existing MCP connections | Clean rename, no backwards compat needed (plugin is pre-release) |
| Protocol too aggressive / saves noise | Start aggressive, tune down later based on experience |
| Context output too large after many memories | `limit` parameter caps output; default 10 for hooks, 20 for MCP tool |
| Session summaries accumulate over time | Rotation policy: keep only 3 most recent per repo, auto-delete older |
| Both engram and qwick-memory active simultaneously | Document that engram should be disabled when qwick-memory is active |
