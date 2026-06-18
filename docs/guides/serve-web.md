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

To enable semantic (hybrid) file search in the search box, point
`--embed-cmd` at your embedder (see [Search the graph](#search-the-graph)):

```bash
comemory serve --embed-cmd 'comemory-embed.sh' --open
```

## Options

| Flag | Purpose |
|------|---------|
| `--repo <REPO>` | Restrict the graph to one repo label (as passed to `index-code --repo`). |
| `--port <PORT>` | Loopback port to bind; `0` (default) selects an ephemeral port whose URL is printed at startup. |
| `--read-only` | Reject backend writes — `PUT /api/file` returns 405. The editor is read-only regardless of this flag (see [The source panel](#the-source-panel-read-only)). |
| `--root <REPO=PATH>` | Override a repo's working-tree root as `<repo>=<abs-path>` (repeatable). Required for repos indexed before the v7 schema captured the root. |
| `--embed-cmd <CMD>` | Embed command that upgrades the search box from lexical to hybrid (semantic). Run as `sh -c <cmd>`, reads the query on stdin, must emit `{"embedding":[..]}`. Mirrors `COMEMORY_EMBED_CMD`. Unset → lexical search only. See [Search the graph](#search-the-graph). |
| `--open` | Open the printed URL in the default browser after binding. |

The session-token URL is passed to the system opener, so it is briefly
visible to other local users (e.g. via `ps`); skip `--open` and paste the
URL yourself if that matters.

## Navigate the 3D graph

The code graph renders in 3D ([`react-force-graph-3d`]). The camera and
labels respond to direct manipulation:

- **Orbit / rotate** — drag to rotate the graph about the X and Y axes.
- **Zoom** — scroll to move the camera in and out.
- **Hover for the full path** — hovering a node reveals its complete
  `<repo>:<path>` file path, so long paths that don't fit on a label are
  still readable.
- **Gated text labels** — persistent labels are deliberately sparse to keep
  the scene legible: they show only for the **selected node**, its
  **immediate neighbors**, and the **top ~30 nodes by PageRank**. Select a
  node to surface its neighborhood's labels.

[`react-force-graph-3d`]: https://github.com/vasturiano/react-force-graph

## The source panel (read-only)

Selecting a node loads its source into a [CodeMirror] panel for reading. The
editor is **read-only**: there is no Save action, and edits are never written
back to disk from the browser. This is true whether or not `serve` was
started with `--read-only`.

`--read-only` now affects only the backend: it makes the `PUT /api/file`
endpoint return `405 Method Not Allowed`. Because the editor itself no longer
offers a write path, the flag is a backstop for scripts or other clients that
might still call the mutating endpoint directly.

[CodeMirror]: https://codemirror.net/

## Read-only mode

```bash
comemory serve --read-only
```

Use read-only mode when you want to guarantee no writes reach disk through
the backend — for demos, shared machines, or untrusted sessions. It makes
`PUT /api/file` return `405`. The in-browser source panel is already
read-only without the flag (see [The source panel](#the-source-panel-read-only)),
so `--read-only` only matters for clients that bypass the SPA and call the
mutating endpoint directly.

## Search the graph

The search box does **natural-language file search**, not a client-side
substring filter. As you query, the SPA calls the backend search route and
ranks whole files by relevance:

```bash
curl "http://127.0.0.1:8787/api/search?q=parse%20frontmatter&k=10"
```

```json
{
  "query": "parse frontmatter",
  "mode": "lexical",
  "hits": [
    {
      "node_id": "file:comemory:src/memory/frontmatter.rs",
      "repo": "comemory",
      "path": "src/memory/frontmatter.rs",
      "score": 0.91,
      "top_symbol": "Frontmatter"
    }
  ]
}
```

`q` is the query and `k` (optional) the result cap. Hits are file-level —
matching symbols are coalesced to their parent file, the highest-scoring
symbol becomes `top_symbol`, and `node_id` is the same `file:<repo>:<path>`
id `GET /api/graph` emits, so a hit selects directly in the graph.

- **Lexical by default** — `mode` is `"lexical"` and ranking is BM25 over
  symbol / snippet / path tokens (the same code-search engine as
  `comemory search-code`).
- **Hybrid with `--embed-cmd`** — start `serve --embed-cmd <CMD>` (or set
  `COMEMORY_EMBED_CMD`) to add a semantic vector leg fused with the lexical
  one; `mode` then reports `"hybrid"`. The contract matches the TUI: the
  command is run as `sh -c <cmd>`, reads the query on stdin, and must emit
  `{"embedding":[..]}` on stdout.
- **Graceful degrade** — if the embed command fails, the request falls back
  to lexical (still returns ranked hits, reports `"lexical"`, and never
  `5xx`s). An empty query or empty index returns `200` with no hits.

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
