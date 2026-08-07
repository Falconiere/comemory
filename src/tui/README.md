# tui/

**What belongs here:** the read-only interactive terminal explorer
(`comemory tui`) — pure UI state (`app`), the key map (`event`), a
dedicated-thread DB-worker that owns the single SQLite connection
(`worker`), the render-side bridge between state and the worker (`search`),
preview-text formatting (`preview`), and the RAII terminal lifecycle guard
(`terminal`). The orchestrator in `src/tui.rs` drives an async `EventStream` +
`tokio::select!` loop over all of them. Nothing here mutates the index.

**What does NOT belong here:** pure rendering. Every ratatui widget lives in
the sibling `view/` module (`view::layout`/`view::list`/`view::preview`), kept
separate so layout is snapshot-testable against a `TestBackend` independent of
state and IO. The `COMEMORY_EMBED_CMD` shell-out also lives outside this
folder, in the shared single-file `embed.rs` module, since `serve` consumes it too.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `app.rs` | `Tab` | Pure UI state for the read-only explorer |
| `event.rs` | `map_key` | Pure key-to-`Action` mapping |
| `preview.rs` | `preview_text` | Preview text for the selected row (pure formatting) |
| `search.rs` | `build_request` | Render-side bridge between `App` state and the DB-worker |
| `terminal.rs` | `Restore` | RAII terminal lifecycle guard |
| `worker.rs` | `Request` | The DB-worker: owns the single SQLite connection and serves search requests |

`view/` (pure ratatui widgets) is documented in its own `README.md` per the
guardrails nested-folder rule.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/tui.rs` (`pub mod
<name>;`) and callers import concrete paths.
