---
name: comemory-save
description: Persist a durable memory to comemory for this repo — a decision made, a bug root-caused, a convention agreed, or a non-obvious discovery. Save PROACTIVELY when these happen, not only when asked. Scoped to the current git repo.
---

# comemory Save

Persist what's worth remembering across sessions. Be proactive — save when the
moment happens, don't wait to be asked.

## When

- A **decision** is made (and why).
- A **bug** is root-caused (cause + fix).
- A **convention** is agreed.
- A non-obvious **discovery / pattern** surfaces.

Don't save what the repo already records (code structure, git history) or what
only matters to this one conversation.

## How

Feed the body on **stdin via a quoted heredoc** so multi-line text needs no
escaping (the wrapper passes it to `comemory save -`):

```bash
"${CLAUDE_PLUGIN_ROOT}/scripts/comemory.sh" save \
  --kind decision --quality 4 --tags "auth,jwt" <<'BODY'
Chose RS256 over HS256 for service tokens so we can rotate the public key
without redeploying verifiers. HS256 would force a shared-secret rollout.
BODY
```

- `--kind` one of `decision|bug|convention|discovery|pattern|note` (default
  `note`).
- `--quality` 1–5 (default 3).
- `--tags` comma-separated.
- `--supersedes <id[,id...]>` when this replaces an earlier memory.

The wrapper auto-scopes `--repo` to the current git repo. If the response JSON
reports `"duplicate_of": "<id>"`, a near-identical memory already exists — tell
the user (offer `--supersedes <id>`) rather than blindly re-saving.
