---
name: memory
description: ALWAYS ACTIVE — Centralized memory protocol for cross-repository knowledge. Save decisions, bugs, conventions, and discoveries proactively.
---

## qwick-rag Memory Protocol

You have qwick-rag memory tools (rag_save, rag_search, rag_list, rag_delete, rag_index, rag_context).

### PROACTIVE SAVE — do NOT wait for user to ask
Call `rag_save` IMMEDIATELY after ANY of these:
- Decision made (architecture, convention, workflow, tool choice)
- Bug fixed (include root cause)
- Convention or workflow established
- Non-obvious discovery or edge case found
- Pattern established (naming, structure, approach)

### SEARCH MEMORY when:
- Starting work on something that might have been done before
- User asks to recall anything
- User mentions a topic you have no context on
- User's first message references a problem or feature

### Memory Types
- `decision` — Architecture, tool, or workflow choices
- `bug` — Bug root causes and fixes
- `convention` — Coding standards, naming patterns
- `discovery` — Non-obvious findings, gotchas
- `pattern` — Established approaches
- `preference` — User or team preferences
- `note` — General knowledge that doesn't fit other types
