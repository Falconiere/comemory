---
name: memory
description: ALWAYS ACTIVE — Centralized memory protocol for cross-repository knowledge. Save decisions, bugs, conventions, and discoveries proactively.
---

## Qwick Memory Protocol

You have qwick-memory tools (qwick_memory_save, qwick_memory_search, qwick_memory_list, qwick_memory_delete, qwick_memory_index, qwick_memory_context, qwick_memory_session_summary).

### PROACTIVE SAVE — do NOT wait for user to ask
Call `qwick_memory_save` IMMEDIATELY after ANY of these:
- Decision made (architecture, convention, workflow, tool choice)
- Bug fixed (include root cause)
- Convention or workflow established
- Non-obvious discovery or edge case found
- Pattern established (naming, structure, approach)
- User preference or constraint learned
- Feature implemented with non-obvious approach

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
- `session-summary` — (used automatically by qwick_memory_session_summary)

### SESSION CLOSE — before saying "done"/"listo":
Call `qwick_memory_session_summary` with: goal, discoveries, accomplished, next_steps, relevant_files.
