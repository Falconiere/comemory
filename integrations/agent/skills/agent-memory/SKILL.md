---
name: agent-memory
description: Recall relevant project knowledge before exploration and save verified corrections, decisions, discoveries, and bug fixes through comemory's MCP tools (or the repo-scoped wrapper as fallback).
---
# Learn through comemory

## MCP tools (preferred)

On an MCP-capable host, comemory registers eleven tools. Claude Code names
them `mcp__plugin_comemory_comemory__<tool>`; call them by that name there,
or by the bare name on any other MCP host:

- `find` — one ranked list across memory, code and documents. Start here.
- `search` — memory-only ranked search.
- `search_code` — lexical code-symbol search.
- `context` — full memory bodies plus linked code for a key. Its `k` limits
  result count, not a token budget.
- `show` — one memory in full (body, frontmatter, activation, references).
- `list` — list memories, filterable.
- `edges` — search the relation graph (supersedes, imports, references).
- `repos` — indexed code repositories and their freshness.
- `recall_status` — tracked recalls awaiting a verdict, verdicts, saves.
- `save` — write a memory.
- `feedback` — record a verdict on a prior recall.

The loop:

1. **Recall.** Call `find` with `k: 3` before exploring — every call scoped to
   this repo unless a cross-repo answer is actually needed. If a hook already
   injected memory IDs, do not repeat `find`: call `show` only for the IDs you
   need to inspect.
2. **Judge.** For hits from a tracked recall with a `query_id`, call `feedback`
   with `used` or `irrelevant` ids for those you judged. Untracked hook hints
   require no feedback. Set `confirmed_by_user: true` only when the user
   themselves stated the verdict — everything else is implicit. Because
   `recall_status` is aggregated over a repository and time window, the Stop
   hook can only give an at-most-once shared-activity advisory; it cannot
   attribute an unjudged recall to one session. Empty-result queries have no
   memory IDs to judge and are omitted from that advisory.
3. **Verify, then save.** Work and verify with real inputs; derive *why*
   from the interaction and observed results, not from the diff alone.
   Compare against recalled memories, then `save` explicit user corrections,
   established decisions, verified discoveries, and validated fixes, with
   evidence and file references. Use `supersedes` for outdated knowledge —
   never a silent duplicate.
4. **Cite carefully.** `show` a memory before citing it, so the citation
   matches the current body, not a stale recollection of it.

Skip routine change summaries, copied documentation, speculation, and
unverified success claims. A recall miss does not require inventing a
lesson. Save facts promptly; for proven recurring procedures, use the
project-skills workflow, patching an existing skill before creating another.

## Wrapper fallback

Hosts without MCP, and the retrieval-quality loop's `maintain`, use the
published wrapper for the active host:

- Claude Code: `"${CLAUDE_CONFIG_DIR:-$HOME/.claude}/comemory/comemory.sh"`
- Codex: `"${CODEX_HOME:-$HOME/.codex}/comemory/comemory.sh"`

SessionStart publishes that path. Do not use plugin-root environment variables
in agent shell commands. The wrapper scopes to the canonical Git repository,
shared across worktrees. `--repo NAME` overrides it explicitly.

1. Announce the repository scope and run `comemory.sh search "targeted topic"`.
2. Work and verify with real inputs. A diff shows what changed; derive why from
   the interaction and observed results.
3. Compare reusable lessons against recalled memories before saving. Save
   explicit user corrections, established decisions, verified discoveries, and
   validated fixes. Include evidence and relevant file references.
4. Run `comemory.sh save "title" "lesson and evidence" --kind bug` (or decision,
   convention, discovery, pattern, note). Use `--supersedes ID` for outdated
   knowledge. A duplicate warning does not mean the save failed.
5. When recalled knowledge helped, run `comemory.sh feedback QUERY_ID --used ID`.
   Query IDs come from search output, including `--json`.

Raw wrapper feedback uses the CLI's manual provenance. Prefer MCP `feedback`
for agent-inferred verdicts so they remain implicit; use the wrapper fallback
when the verdict is explicitly manual or MCP is unavailable.

`search`, `context`, `list`, and `save` accept `--json`. The wrapper also exposes
`delete`, `search-code`, `index-code`, `graph`, and local retrieval maintenance.
Use `comemory.sh --help` for the full interface. Installation is explicit:
`comemory install claude` or `comemory install codex`. Git indexing hooks remain
an independent choice: `comemory install-hooks`.
