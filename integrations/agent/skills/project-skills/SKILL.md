---
name: project-skills
description: "ALWAYS ACTIVE — Write a project skill to .toolu/skills/ after a proven class-level workflow; patch first. Archive each unused skill, never delete."
---
# Project skills

Procedural memory for **this repo**. Facts stay in comemory (`agent-memory`). Procedures live as `SKILL.md` files the SessionStart index lists by name+description; `Read` the file to load the body.

Install with `comemory install claude` or `comemory install codex`. Committed skills in a clone are indexed when the integration starts.

## Wrapper — `skills.sh`

```bash
# Codex
"${CODEX_HOME:-$HOME/.codex}/comemory/skills.sh" <subcmd> …
# Claude Code
"${CLAUDE_CONFIG_DIR:-$HOME/.claude}/comemory/skills.sh" <subcmd> …
```

Repo-checkout fallback: `integrations/agent/skills/project-skills/scripts/skills.sh`.

## Create (mandatory after a proven class-level workflow)

Call `skills.sh create` when ALL of these hold:

- The workflow is a **recurring task type**, not a ticket or session (`fix-issue-123` is a reject).
- It took several tool calls and **succeeded**, or the user corrected the approach and the correction worked.
- No existing project skill covers it — if one is close, **Write/Edit that file** (PostToolUse records the patch) instead of creating a sibling.

Do **not** encode unresolved failures, "tool X is broken", env-missing errors, or one-offs.

```bash
skills.sh create deploy-staging --description "Deploy this repo to staging. Use when shipping a branch to staging." --file /tmp/skill.md
```

`--file` (or stdin) must contain `## When to Use`, `## Procedure`, `## Pitfalls`, `## Verification`. Description ≤ 30 words. Git status is the human gate — no extra confirm.

Announce repo scope the same way as `agent-memory` before creating.

## Load

SessionStart injects at most 20 `- name: description` lines. To follow a procedure, `Read` `<repo>/.toolu/skills/<name>/SKILL.md`. Do not treat the index as the body.

## Curator

A daily Stop hook archives agent-created skills idle 90 days (stale at 30). **Never deletes.** Marketplace `plugins/*/skills/` are never touched. `skills.sh pin` / `restore` / `adopt` / `status` / `curate --dry-run` for manual control.

Disable: `skills.comemory: false` (whole plugin) or `projectSkills.enabled: false`.
