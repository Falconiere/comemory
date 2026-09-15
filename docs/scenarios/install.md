# `comemory install`

Install the bundled comemory skills and hooks through the native agent host's
plugin manager. This is separate from Git reindex hooks (`install-hooks`).

**Runnable tests:** `tests/cli_scenario_install.rs`; native host installation,
upgrade, hook execution and worktree recall: `bash scripts/test-agent-install.sh`.

**HTTP:** none — CLI only.

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`<HOST>`: `claude` or `codex` (aliases `claude-hooks` and `codex-hooks`).

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--dry-run` | off | Preview destinations without writes or host dependencies |
| `--config-dir` | host environment or home directory | Override host configuration location |

## Scenarios

### install-01 Isolated preview

- **Flags:** `--dry-run` `--config-dir` `--json` `--data-dir`
- **Setup:** an empty temporary directory.
- **Command:** `comemory install claude --dry-run --json --data-dir /tmp/memory-preview --config-dir /tmp/host-preview`
- **Expect:** JSON identifies `comemory@comemory`; neither destination is created.
- **Covered by:** `tests/cli_scenario_install.rs::installation_preview_does_not_write_or_require_a_host`
