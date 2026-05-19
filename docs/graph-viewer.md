# Graph viewer

`qwick-memory graph serve` starts a local HTTP server and opens a
browser-based property-graph viewer. Assets are embedded in the binary via
`rust-embed`; no network access is required after installation.

## Quick start

```bash
# Open the viewer (browser opens automatically)
qwick-memory graph serve

# Headless or over SSH (print URL only)
qwick-memory graph serve --no-open

# Pin to a specific port
qwick-memory graph serve --port 7878
```

The server binds to `127.0.0.1` (loopback) by default.  To expose it on a
non-loopback interface, pass both `--host <addr>` and `--bind-public` (the
flag acknowledges that the viewer is read-only but unauthenticated).

## REST endpoints

| Endpoint | Description |
|---|---|
| `GET /api/seed` | Initial graph payload: `Memory`, `Repo`, `Author`, `Tag` nodes and their edges |
| `GET /api/expand?id=<node_id>` | One-hop neighbours of a node |
| `GET /api/search?q=<text>` | Full-text node search, returns matching node ids |
| `GET /api/node/:id` | Full node detail: frontmatter fields and edge list |

All endpoints return JSON. The viewer itself is served from `/`.

## Manual smoke checklist

After running `qwick-memory graph serve`, verify by eye:

1. `qwick-memory save "test memory body"` (or use any existing memory).
2. `qwick-memory graph serve` — note the printed URL; the browser opens automatically.
3. Default view shows the memory layer (`Memory`, `Repo`, `Author`, `Tag`).
4. Double-click a `Memory` node — neighbours appear.
5. Toggle the Code layer — `File` and `Symbol` nodes appear (if `qwick-memory index-code` has been run).
6. Type a tag name in the search box — pick a result; the node centres.
7. Click a `Memory` node — body and edges render in the detail panel.
8. Press `R` — view resets to the memory layer.
9. Ctrl-C — server shuts down cleanly.

## Architecture notes

The `serve/` module holds a shared `Arc<Mutex<Graph>>`, registers the four
read-only REST routes via `axum`, and embeds the Cytoscape frontend from
`frontend/` using `rust-embed`. The server runs in-process on a
`tokio` runtime and shuts down on receipt of `SIGINT` or `SIGTERM`.

See [architecture.md](architecture.md) for the module-level context and
[cli-reference.md](cli-reference.md) for all flags and examples.
