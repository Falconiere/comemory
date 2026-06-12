---
name: comemory-search-code
description: Search this repo's indexed code by symbol or intent (BM25 + graph priors), e.g. "where is X defined", "what handles auth". Prefer over raw grep for locating where a symbol or behavior lives before editing.
---

# comemory Search Code

Locate where a symbol or behavior lives using comemory's ranked code index.

## When

- Before editing, to find the symbol/file that owns a behavior.
- For "where is X" / "what handles Y" — ranked results beat raw grep.

## How

```bash
"${CLAUDE_PLUGIN_ROOT}/scripts/comemory.sh" search-code "<symbol or intent>" --json
```

Optional: `--lang rust|typescript|javascript|python|go` (aliases `rs|ts|tsx|js|jsx|py`)
to constrain language, `--k N` to cap results.

Returns `hits[]` with `repo`, `path`, `symbol`, `kind`, `lang`, `lines`, and a
`score`. Open the top hits by `path` + `lines`. If results are empty the index
may be stale — suggest `comemory index-code`. On
`{"comemory":"unavailable",...}` fall back to ast-grep / Grep.
