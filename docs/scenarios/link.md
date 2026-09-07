# `comemory link`

Optional local cache of `repo label → workspace id` under
`[sync.repos]` in `config.toml`. **Not** the source of truth — the
server GitHub App allowlist wins at push and import time.

**Runnable tests:** `tests/cli__link.rs`

**HTTP:** none (`transport: "cli-only"`)

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--workspace` | config / personal | Workspace id to cache against |
| `--repo` | _(required)_ | Repo label (`owner/name` or basename) to cache |

## Scenarios

### link-01 Repo required

- **Flags:** `--workspace` `--repo`
- **Command:** `comemory link --workspace ws_test`
- **Expect:** usage error naming `--repo`.
- **Covered by:** `tests/cli__link.rs::link_requires_repo`

### link-02 Writes config override

- **Flags:** `--workspace` `--repo`
- **Command:** `comemory --json link --workspace ws_org --repo codasignal/foo`
- **Expect:** exit 0; `config.toml` gains the mapping; JSON echoes both.
- **Covered by:** `tests/cli__link.rs::link_writes_sync_repos_override`
