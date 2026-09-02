# `comemory install-hooks`

Write `post-commit` / `post-merge` / `post-checkout` into a git repo so
those events trigger `comemory index-code` in the background.

**Runnable tests:** `tests/cli__hooks.rs`, `tests/api__install_hooks.rs`,
`tests/cli_scenario_hooks.rs`

**HTTP:** `POST /api/v1/hooks/install` — covered by `tests/serve_scenario_hooks.rs`

Global flags `--json` and `--data-dir` apply (`--data-dir` is unused).
See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--repo` | `.` | Git repo root to install into |
| `--force` | off | Overwrite existing hook files. Without it, pre-existing hooks are refused |

## Scenarios

### install-hooks-01 Fresh repo

- **Flags:** `--repo` `--json`
- **Setup:** `git init` with a test identity
- **Command:** `comemory install-hooks --repo /path/to/repo --json`
- **Expect:** three hook files exist; `hooks --json` reports them installed.
- **Covered by:** `tests/cli_scenario_hooks.rs`

### install-hooks-02 Force

- **Flags:** `--force`
- **Setup:** a hand-written `post-commit`
- **Command:** `comemory install-hooks --repo /path/to/repo --force`
- **Expect:** without `--force`, refuse to clobber; with `--force`, overwrite.
- **Covered by:** `tests/api__install_hooks.rs`, `tests/cli_scenario_hooks.rs`
