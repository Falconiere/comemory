# `comemory install-hooks`

Write `post-commit` / `post-merge` / `post-checkout` / `post-rewrite` into a
git repo so every HEAD move runs `comemory sync --action auto --path <checkout>`
in the background: the checkout is indexed, every other stale hooked repo is
refreshed, and — logged in — everything syncs. The CLI then runs the shipped hook body once in the repo (from the binary,
never a file in the repo's hooks directory), so the repo is registered and
synced now rather than at its next commit; `--json` reports that as `kicked`
(`false` only when `bash` could not be run — the install stands either way).
The HTTP route writes the hooks without the kick.

**Runnable tests:** `tests/cli__hooks.rs`, `tests/api__install_hooks.rs`,
`tests/cli_scenario_hooks.rs`, `tests/cli__sync_auto.rs`

**HTTP:** `POST /api/v1/hooks/install` — covered by `tests/serve_scenario_hooks.rs`

Global flags `--json` and `--data-dir` apply (`--data-dir` is handed to the
kicked pass).
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
- **Expect:** four hook files exist; `hooks --json` reports them installed.
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
- **Expect:** every hook is rewritten with the current body (which hands the
  checkout to `sync --action auto`, where the label is the main worktree's)
  and no longer reads as outdated.
  Re-running over an already-current hook is a no-op, not an error.
- **Covered by:** `tests/api__install_hooks.rs::run_repairs_an_outdated_comemory_hook_without_force`,
  `tests/api__install_hooks.rs::run_is_idempotent_over_an_up_to_date_comemory_hook`,
  `tests/cli_scenario_hooks.rs`

### install-hooks-04 The first pass starts without a commit

- **Flags:** `--repo` `--json`
- **Setup:** a never-indexed repo; `comemory` reachable by the hook
- **Command:** `comemory install-hooks --repo /path/to/repo --json`, run from outside the repo
- **Expect:** `"kicked": true`, four hooks in `installed`, and `comemory repos --json` lists the repo with a `last_head` once the background pass finishes.
- **Covered by:** `tests/cli__sync_auto.rs::install_hooks_kicks_the_first_pass_for_a_never_indexed_repo`
