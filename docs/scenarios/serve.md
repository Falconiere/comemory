# `comemory serve`

Loopback HTTP API (`/api/v1`) for consoles, agents, and scripts. Binds
127.0.0.1. `--json` prints the startup banner (`url`, `token`, `port`).

The embedded web viewer was removed; this command is the API server.

**Runnable tests:** `tests/cli__serve.rs`, `tests/serve__routes__*`

**HTTP:** none — this command *is* the server (`transport: "cli-only"` in `GET /api/v1/commands`; asserted by `tests/api__parity.rs`)

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--repo` | unset | Default repo label for reads that accept a `repo` filter. Overridden by a request `repo` param or `X-Comemory-Repo` |
| `--port` | `0` | Loopback port. `0` = ephemeral; URL printed at startup |
| `--read-only` | off | Every mutating `/api/v1` route answers 405 `read_only` |
| `--root` | unset | Repeatable `<repo>=<abs-path>` working-tree override |
| `--embed-cmd` | `COMEMORY_EMBED_CMD` | `sh -c` embedder for server-side vectorize routes. Unset → those routes 503 |
| `--allow-path` | unset | Repeatable extra directory path-taking mutating routes may touch |

## Scenarios

### serve-01 Startup banner

- **Flags:** `--json` `--port`
- **Setup:** indexed demo repo
- **Command:** `comemory serve --json --port 0`
- **Expect:** JSON banner with `url` / token; process listens on loopback.
- **Covered by:** `tests/cli__serve.rs`

### serve-02 Read-only

- **Flags:** `--read-only`
- **Command:** `comemory serve --read-only --json`
- **Expect:** mutating `/api/v1` routes return 405.
- **Covered by:** `tests/cli__serve.rs` / `tests/serve__routes__*`

### serve-03 Repo, root, allow-path, embed-cmd

- **Flags:** `--repo` `--root` `--allow-path` `--embed-cmd`
- **Command:** `comemory serve --repo demo --root demo=/abs/path --allow-path /golden --embed-cmd 'cat >/dev/null'`
- **Expect:** default repo applies to unscoped reads; `--root` overrides a
  missing v7 root; `--allow-path` is the extra containment root for
  path-taking jobs; unset embed-cmd → 503 on reembed.
- **Covered by:** `tests/cli__serve.rs`, `tests/serve__routes__*`
