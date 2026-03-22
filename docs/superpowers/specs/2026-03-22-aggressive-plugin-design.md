# Aggressive Plugin Discoverability & Behavioral Reinforcement

**Date:** 2026-03-22
**Status:** Approved
**Goal:** Make qwick-memory plugin more aggressive (saves and searches reliably) and more discoverable (for both Claude and users).

## Problem

The plugin has the right tools and protocol text, but Claude doesn't reliably:
1. **Search** memory before answering questions about prior work, PRs, decisions, or history
2. **Save** proactively after decisions, bug fixes, discoveries, or conventions
3. The MCP `instructions=PROTOCOL` text gets buried in system context — Claude pays more attention to tool descriptions and hook outputs

## Strategy: Multi-Layer Reinforcement

Spread behavioral triggers across multiple high-priority surfaces in Claude's context, each concise and non-redundant:

| Surface | Priority for Claude | Purpose |
|---------|-------------------|---------|
| Tool descriptions | Highest (adjacent to tool-call decisions) | WHEN to call each tool |
| PROTOCOL (MCP instructions) | Medium (system context) | Decision tree for every message |
| SessionStart hook output | High (top of session) | Compact 3-line decision guide |
| Tool response text | High (immediate feedback) | Next-action hints after each call |
| Marketplace metadata | N/A (user-facing) | Value proposition for discovery |

## Section 1: Tool Description Rewrites

Every tool description leads with **trigger conditions** — when Claude should call the tool — before the functional description.

### `qwick_memory_search`

```
Search memories by semantic similarity.

CALL THIS TOOL FIRST when:
- User asks about prior work, PRs, decisions, bugs, or project history
- User asks "what do you know about...", "remember...", "what did we do..."
- Starting work that might overlap with previous sessions
- User's first message references a feature, problem, or project area
- You lack context on a topic the user is discussing

-> Always search BEFORE answering from general knowledge. Memory has project-specific context you don't.

Default: search all repos. Use the repo filter only when the user explicitly scopes to one project.
```

### `qwick_memory_save`

```
Save a memory to the knowledge base. Called proactively — do NOT wait for user to ask.

CALL THIS TOOL AFTER:
- Making an architecture, convention, or workflow decision
- Fixing a bug (include root cause and fix)
- Discovering a non-obvious gotcha or edge case
- Establishing a pattern or convention
- Learning a user preference or constraint
- Implementing a feature with a non-obvious approach

Self-check after every task: "Did I just decide, fix, learn, or establish something?" If yes → save NOW.
```

### `qwick_memory_context`

```
Get recent memories for context. Includes latest session summary + recent memories.

CALL THIS when:
- Starting a new session (if SessionStart hook didn't fire)
- Resuming work after a pause
- User asks for a status update or "where were we?"
```

### `qwick_memory_session_summary`

```
Save a structured session summary. MUST be called before ending a session.

CALL THIS when:
- User says "done", "listo", "that's it", "thanks", or signals session end
- Before context compaction
- After completing a significant milestone
```

### Other tools (`list`, `index`)

No description changes — these are utility tools that don't need behavioral triggers.

### `qwick_memory_delete`

Add a brief confirmation hint to the response:
```
Deleted memory {id}. Removed from disk and vector index.
```

## Section 2: PROTOCOL Restructure — Decision Tree

Replace the current flat bullet list with a sequential decision tree that Claude evaluates on every user message.

```
## Qwick Memory — ACTIVE PROTOCOL

You have qwick-memory tools. This protocol is MANDATORY.

### DECISION TREE — follow on EVERY user message:

**Step 1: Should I SEARCH first?**
User message contains ANY of these → call `qwick_memory_search` BEFORE responding:
- Questions: "what", "why", "how", "remember", "recall", "what did we"
- References: PR, issue, bug, feature, decision, architecture
- Implicit: user assumes you have context you don't have
- First message: always search with project/topic keywords
→ If unsure, SEARCH. Cost of an unnecessary search is near zero.
  Cost of missing context is a wrong answer.
→ For "where were we?" or status requests, prefer `qwick_memory_context`.

**Step 2: Should I SAVE after?**
You just did ANY of these → call `qwick_memory_save` IMMEDIATELY:
- Made a decision (architecture, convention, workflow, tool choice)
- Fixed a bug (save root cause + fix)
- Discovered something non-obvious (gotcha, edge case, quirk)
- Established a pattern or convention
- Learned a user preference or constraint
- Created/updated an artifact (PR, doc, config, Jira, Notion)
→ If unsure, SAVE. A redundant memory is better than a lost insight.

**Step 3: Is this session ending?**
User signals completion → call `qwick_memory_session_summary`:
- "done", "listo", "thanks", "that's it", "bye"
- Context compaction imminent
- Major milestone completed
```

Key changes from current PROTOCOL:
- Decision tree vs flat list — forces sequential evaluation
- Bias toward action — "if unsure, SEARCH" / "if unsure, SAVE"
- Removed memory types list — already in tool descriptions, no duplication

## Section 3: Enhanced SessionStart Hook Output

Add a compact 3-line decision guide footer to the existing context output. ~30 tokens, high-attention position.

**New output format:**

```
## Qwick Memory — Session Context

### Last Session
[session summary content...]

### Recent Memories
[recent memories list...]

---
Memory Protocol Active:
-> SEARCH before answering questions about prior work, PRs, decisions, or history
-> SAVE after decisions, bug fixes, discoveries, conventions, preferences
-> SUMMARIZE before ending session
```

**Implementation:** Add the 3-line footer as static `echo` statements in `session-start.sh` (not in the `qwick_memory_context` Python function). The footer is hook-specific reinforcement — it should not appear when `qwick_memory_context` is called interactively mid-session.

No changes to PreCompact/PostCompact hooks — they already serve their purpose.

## Section 4: Tool Response Hints

Brief contextual hints in tool responses that nudge Claude toward the next appropriate action. ~15 tokens each.

### `qwick_memory_save` — success

```
Saved memory {id} ({type}). Embedded and indexed for vector search.
-> This memory is now searchable by semantic similarity across all future sessions.
```

### `qwick_memory_save` — duplicate detected

```
Memory already exists: {id}. Content was previously saved.
-> No action needed. The memory is already indexed and searchable.
```

### `qwick_memory_search` — results found

```
[results with similarity scores...]
-> Results ranked by semantic similarity. Use these memories to inform your response.
```

### `qwick_memory_search` — no results

```
No semantically similar memories found.
-> If you learn something new about this topic, save it with qwick_memory_save so future searches can find it.
```

### `qwick_memory_index` response

```
Indexed: {new} new, {updated} updated, {deleted} deleted. Total: {count} vectors.
-> Vector index rebuilt. All memories are now searchable by semantic similarity.
```

### `qwick_memory_session_summary` response

```
Saved session summary {id}. Embedded and indexed for vector search.
-> Session context preserved for next time.
```

### `qwick_memory_delete` response

```
Deleted memory {id}. Removed from disk and vector index.
```

No hints on `list` — utility tool that doesn't need behavioral reinforcement.

## Section 5: Marketplace & Plugin Metadata

Rewrite descriptions to communicate behavioral value, not just capabilities.

### marketplace.json description

```
Persistent developer memory across repos — automatically saves decisions, bugs, conventions, and discoveries. Semantic vector search recalls prior work so Claude never forgets what you've built.
```

### plugin.json description

```
Persistent developer memory with semantic vector search — Claude automatically saves and recalls decisions, bugs, conventions, and discoveries across repositories.
```

## Files to Modify

| File | Changes |
|------|---------|
| `src/qwick_memory/server.py` | PROTOCOL rewrite, tool descriptions, response hints |
| `scripts/session-start.sh` | Add 3-line decision guide footer (static echo) |
| `skills/memory/SKILL.md` | Trim to minimal pointer — defer to tool descriptions and PROTOCOL |
| `.claude-plugin/marketplace.json` | New description |
| `.claude-plugin/plugin.json` | New description |

## Token Budget

| Surface | Current tokens (approx) | New tokens (approx) |
|---------|------------------------|---------------------|
| PROTOCOL | ~280 | ~200 (shorter, no duplication) |
| Tool descriptions (total) | ~120 | ~350 |
| SessionStart footer | 0 | ~30 |
| Tool response hints | ~20 | ~80 |
| **Total delta** | | **+~240 tokens** |

Net increase of ~240 tokens spread across high-priority surfaces. The PROTOCOL itself shrinks by ~80 tokens because we removed the duplicated memory types list.

**Verification:** After implementation, count actual tokens in PROTOCOL + tool descriptions + response hints to confirm the budget holds.

## Design Decisions

1. **Tool descriptions are the primary injection point** — Claude's tool-calling model weighs descriptions heavily when deciding which tool to invoke. This is where "CALL THIS FIRST" has the most impact.

2. **Decision tree > bullet list** — A sequential "Step 1, Step 2, Step 3" structure is easier for Claude to follow than a flat list of "when X, do Y" rules.

3. **Bias toward action** — "If unsure, SEARCH/SAVE" ensures the default is to use the tools. A redundant search or save is cheap; a missed one loses context.

4. **Response hints close the loop** — After each tool call, Claude gets a nudge toward the next action (search → use results; save → it's indexed; no results → save later).

5. **No changes to PreCompact/PostCompact** — They already work. Adding more surfaces there would increase tokens without proportional benefit.

6. **SKILL.md trimmed to minimal pointer** — The current SKILL.md has its own copy of the protocol (flat bullet list), which would conflict with the new decision tree. Trim it to a 3-line description that defers to tool descriptions and PROTOCOL for behavioral rules. This avoids contradictions and saves tokens.

7. **SessionStart footer is shell-only** — The 3-line decision guide is added as static `echo` in `session-start.sh`, not in the `qwick_memory_context` Python function. This keeps the context tool clean for interactive mid-session use.

8. **Default search is cross-repo** — `qwick_memory_search` searches all repos by default (no repo filter). This ensures cross-cutting decisions (deployment conventions, shared patterns) are discoverable. The repo filter is opt-in for when the user explicitly scopes a search.

## Testing Notes

Existing tests use `in` checks on response strings (e.g., `"Saved" in result`). The new response hints append text after the existing strings, so `in` assertions remain valid. However, review all test assertions in `tests/test_server.py` and `scripts/e2e-test.sh` after implementation to confirm nothing breaks.
