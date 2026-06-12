# comemory — Claude Code plugin

Surfaces [comemory](../../README.md)'s memory + code search inside a Claude Code
session by wrapping the existing `comemory` CLI. Zero in-process LLM, no MCP — a
thin shell + skills layer over the binary.

## What it does

- **Auto-recall on session start** — a `SessionStart` hook injects a compact
  digest of this repo's saved memories so the agent starts with prior context.
- **`comemory-recall` skill** — pull memories relevant to a query (decisions,
  bugs, conventions, discoveries).
- **`comemory-search-code` skill** — ranked code search (BM25 + graph priors).
- **`comemory-save` skill** — persist a decision/bug/convention/discovery,
  proactively.

Every call is scoped to the current git repo (git-root basename, overridable
with `COMEMORY_REPO`).

## Prerequisite

The `comemory` binary must be on `PATH`:

```bash
cargo install comemory          # or: brew install Falconiere/tap/comemory
```

If it's absent, the plugin fails soft — the hook injects nothing and skills tell
you to install it; sessions are never broken.

## Install

Install from the marketplace (the plugin root is this directory,
`integrations/claude-code/`). The `.claude-plugin/plugin.json` manifest,
`hooks/hooks.json`, and `skills/` are picked up automatically.

## Optional: vector ranking

v1 is lexical-only. To improve ranking with embeddings, supply vectors yourself
via comemory's BYO-vector flags — see `scripts/comemory-embed.sh` in the repo
root and the main README's "BYO-Vector workflow". The plugin's skills stay
lexical by default.

## Layout

```
.claude-plugin/plugin.json   manifest
hooks/hooks.json             SessionStart registration
hooks/session-start.sh       auto-recall digest (fail-soft)
scripts/comemory.sh          shared wrapper: git-scope + missing-binary guard
skills/comemory-recall/      recall memories for a query
skills/comemory-search-code/ ranked code search
skills/comemory-save/        persist a memory (body via heredoc)
tests/                       bats tests against the real binary
```

## Tests

```bash
just claude-plugin-test
```

Runs the `bats` suite against the real `comemory` binary (seeding a temp
`COMEMORY_DATA_DIR` + throwaway git repo). Skips with a notice if `bats` is not
installed; runs outside the Rust quality gate.
