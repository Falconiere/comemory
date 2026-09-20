---
name: architecture-map
description: Keep this repo's component-level architecture model current — scaffold it from the code index, enrich it with real names and responsibilities, save it, and check it for drift.
---
# Map this repo's architecture

## When to Use

Someone asks how the repository is laid out or wants a diagram of it, a
`comemory architecture check` reports drift, or a change added, removed, or
moved a whole module. Not on every session: the model is a durable artifact,
refreshed when the structure moves, not when a line changes.

## Procedure

1. Scaffold from what is actually indexed — never invent the shape:

       comemory architecture scaffold --repo <scope> --json > /tmp/arch.json

   An empty or failing scaffold means the repo is not indexed yet; run
   `comemory index-code --repo <scope> --path .` first.

2. Enrich `/tmp/arch.json` by reading the code, not by guessing:
   - `name`: what the component is called by the people who work on it.
   - `summary`: one line, at most 280 characters, on what it is responsible
     for. Folder READMEs and module docs are the best source.
   - `kind`: `module`, `service`, `layer`, `store`, or `external`. An
     `external` component describes a dependency outside the repo and must
     carry an empty `members` list.
   - `groups`: add them when components form layers, then set each
     component's `group`.
   - `edges`: keep the mined `imports` / `co_changed` edges; add `calls`,
     `depends`, `reads`, `writes` or `publishes` edges you can point at code
     for.
   - Set `"source": "agent"`. Leave `schema`, `repo`, and every `members`
     path alone unless a component is genuinely wrong.

3. Save it. The save validates every member path against the code index and
   refuses the model if one matches no indexed file:

       comemory architecture save /tmp/arch.json --repo <scope>

4. Check drift before trusting an existing model, and after any restructuring:

       comemory architecture check --repo <scope> --json

   `unmapped` means a directory no component covers; `stale_members` means a
   member path no longer exists (visible after `comemory index-code --mode
   full`, since incremental indexing never revisits a deleted file);
   `missing_edges` means a mined relation the model omits.

5. Show it when a human wants to look at it:

       comemory architecture show --repo <scope> --format mermaid

## Notes

- The model is stored as one memory tagged `architecture`, so it syncs and the
  console can render it. A new save supersedes the previous model; the old one
  stays readable.
- `comemory architecture learn --command '<agent command>'` runs this same loop
  with another agent doing step 2. It is for a human to start, not for you to
  call on your own.
- Keep it small. At most 200 components and 32 KiB; a model that lists every
  file is the file graph again, and `comemory graph` already draws that.
