---
name: comemory-recall
description: Recall this repo's saved comemory memories (decisions, bugs, conventions, discoveries, patterns) relevant to a query. Use BEFORE answering anything that depends on prior decisions for this repo, or when the user references past work ("what did we decide", "why is this like this", "have we seen this bug").
---

# comemory Recall

Pull prior memories for the current repo before reasoning from scratch.

## When

- Before answering a question whose answer depends on a past decision, bug,
  convention, or discovery in this repo.
- When the user references earlier work or asks "why" about existing code.

## How

Run the wrapper (it scopes to the current git repo automatically):

```bash
"${CLAUDE_PLUGIN_ROOT}/scripts/comemory.sh" context "<the user's intent as a query>" --json
```

`context` returns `memories[]`, `code_refs[]`, and `relations[]`. If it yields
nothing useful, fall back to broader memory search:

```bash
"${CLAUDE_PLUGIN_ROOT}/scripts/comemory.sh" search "<query>" --json
```

Cite the returned memory `id`s when you use them, so the user can trace the
source. If the wrapper prints `{"comemory":"unavailable",...}`, tell the user to
`cargo install comemory` and proceed without recall.
