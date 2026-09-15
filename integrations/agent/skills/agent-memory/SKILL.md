---
name: agent-memory
description: Recall relevant project knowledge before exploration and save verified corrections, decisions, discoveries, and bug fixes through the repo-scoped comemory wrapper.
---
# Learn through comemory

Use the published wrapper for the active host:

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

Skip routine change summaries, copied documentation, speculation, and unverified
success claims. A search miss does not require inventing a lesson. Save facts in
comemory promptly; for proven recurring procedures, use the project-skills
workflow, patching an existing skill before creating another.

`search`, `context`, `list`, and `save` accept `--json`. The wrapper also exposes
`delete`, `search-code`, `index-code`, `graph`, and local retrieval maintenance.
Use `comemory.sh --help` for the full interface. Installation is explicit:
`comemory install claude` or `comemory install codex`. Git indexing hooks remain
an independent choice: `comemory install-hooks`.
