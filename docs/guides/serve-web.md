# Run the local web viewer

Browse your indexed code graph and memories in a browser, served entirely
from the `comemory` binary. The single-page app (SPA) is baked into the
binary, so there is nothing to install and no external service to reach —
the server binds to loopback HTTP only.

## Start the viewer

```bash
comemory serve --open
```

This binds an ephemeral loopback port, prints the URL (with a session
token), and opens it in your default browser. Stop the server with Ctrl-C.

To pin a port and scope to one repo:

```bash
comemory serve --repo myrepo --port 8787 --open
```

## Options

| Flag | Purpose |
|------|---------|
| `--repo <REPO>` | Restrict the graph to one repo label (as passed to `index-code --repo`). |
| `--port <PORT>` | Loopback port to bind; `0` (default) selects an ephemeral port whose URL is printed at startup. |
| `--read-only` | Disable all writes — `PUT /api/file` returns 405 and the editor's Save action is hidden. |
| `--root <REPO=PATH>` | Override a repo's working-tree root as `<repo>=<abs-path>` (repeatable). Required for repos indexed before the v7 schema captured the root. |
| `--open` | Open the printed URL in the default browser after binding. |

The session-token URL is passed to the system opener, so it is briefly
visible to other local users (e.g. via `ps`); skip `--open` and paste the
URL yourself if that matters.

## Read-only mode

```bash
comemory serve --read-only
```

Use read-only mode when you want to explore the graph without risking any
writes to disk — for demos, shared machines, or untrusted sessions. The
in-browser editor still loads files but hides Save, and mutating endpoints
reject requests.

## The graph API

`GET /api/graph` returns the file-level code graph that backs the viewer.
It honors optional `rel`, `min_weight`, `limit`, and `offset` query params,
mirroring the `comemory graph` CLI flags.

With **neither** `limit` nor `offset` present, it returns the full
`{ nodes, edges }` graph (the backward-compatible default the embedded SPA
consumes):

```bash
curl "http://127.0.0.1:8787/api/graph?rel=imports"
```

When **either** `limit` or `offset` is present, it returns a paginated
subgraph — a `GraphPage` envelope:

```bash
curl "http://127.0.0.1:8787/api/graph?limit=50&offset=0"
```

```json
{ "nodes": [...], "edges": [...], "limit": 50, "offset": 0, "total": 1234, "has_more": true }
```

Pagination is **edge-windowed**: `total` is the edge count, the page slices
that edge dimension, and `nodes` are derived from the windowed edges'
endpoints. `has_more` is true while more edges remain past the window.

## Repo roots (`--root`)

```bash
comemory serve --root myrepo=/abs/path/to/repo
```

`--root` overrides a repo's working-tree root as `<repo>=<abs-path>` and is
repeatable. It is required for repos indexed before the v7 schema captured
the root, so the viewer can resolve and open files on disk.

## See also

- [CLI reference: `comemory serve`](../cli-reference.md)
- [Getting started](../getting-started.md)
- [Architecture](../architecture.md)
