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
| `--force` | off | Overwrite a hook comemory did **not** write. A hook comemory did write is always refreshed to the current body, with or without this flag |

## Scenarios

### install-hooks-01 Fresh repo

- **Flags:** `--repo` `--json`
- **Setup:** `git init` with a test identity
- **Command:** `comemory install-hooks --repo /path/to/repo --json`
- **Expect:** three hook files exist; `hooks --json` reports them installed.
- **Covered by:** `tests/cli_scenario_hooks.rs`

### install-hooks-02 Force over a foreign hook

- **Flags:** `--force`
- **Setup:** a hand-written `post-commit`
- **Command:** `comemory install-hooks --repo /path/to/repo --force`
- **Expect:** without `--force`, refuse to clobber and leave the hand-written
  file byte-identical; with `--force`, overwrite it.
- **Covered by:** `tests/api__install_hooks.rs`, `tests/cli_scenario_hooks.rs`

### install-hooks-03 Refresh comemory's own outdated hook

- **Flags:** `--repo`
- **Setup:** a repo carrying the hook body comemory shipped before the
  worktree-label rule — it holds the `comemory index-code` marker, so it has
  always reported as installed, while passing the checkout's own basename as
  `--repo` and so registering every `git worktree add` as a new repository.
- **Command:** `comemory install-hooks --repo /path/to/repo`
- **Expect:** all three hooks are rewritten with the current body (which
  derives the label from `--git-common-dir`) and no longer read as outdated.
  Re-running over an already-current hook is a no-op, not an error.
- **Covered by:** `tests/api__install_hooks.rs::run_repairs_an_outdated_comemory_hook_without_force`,
  `tests/api__install_hooks.rs::run_is_idempotent_over_an_up_to_date_comemory_hook`,
  `tests/cli_scenario_hooks.rs`
