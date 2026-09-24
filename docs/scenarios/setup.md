# `comemory setup`

Detect what this machine and this repo still need, show the plan, and apply
the parts you pick. One command in place of the `install` / `install-hooks` /
`index-code` / `index` / `auth login` sequence, and it never redoes work that
is already done.

**Runnable tests:** `tests/cli__setup.rs`, `tests/cli_scenario_setup.rs`

**HTTP:** none — CLI only. Setup writes into the operator's own machine (an
agent host's configuration directory, a repo's git hooks), which a server must
never do on request.

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--yes` | off | Apply every pending step without prompting. Conflicts with `--dry-run` |
| `--dry-run` | off | Report the plan and change nothing. Conflicts with `--yes` |
| `--only` | all steps | Comma-separated step ids to run; every other pending step becomes `skipped` |
| `--skip` | none | Comma-separated step ids to force to `skipped`. Wins over `--only` |
| `--host` | every host on `PATH` | Restrict the `agent-host` step to `claude` or `codex` |
| `--repo` | working directory | Repo root for the repo-scoped steps |

## Steps

The ids `--only` / `--skip` accept, and what each reports as already done:

| id | Satisfied when |
| --- | --- |
| `data-dir` | `comemory.db` exists and is writable |
| `agent-host` | the bundle for this comemory version is installed in the host |
| `git-hooks` | all four reindex hooks are installed in the repo (a repo hooked before `post-rewrite` existed plans the missing one) |
| `index-code` | `comemory repos` reports the repo `fresh` |
| `index-docs` | at least one document source is registered for the repo |
| `reinforce` | `[reinforce] enabled` is on (it ships on) |
| `cloud-auth` | a usable credential is in `auth.json`. Report-only: setup never runs the device flow, it points at `comemory auth login` |

## Scenarios

### setup-01 Interactive default

- **Flags:** _(none)_
- **Setup:** any repo; stdin **not** a terminal (piped, redirected, or CI)
- **Command:** `comemory setup`
- **Expect:** with no terminal to prompt on, the plan is printed along with
  `comemory setup --yes`, and nothing is changed. Exit 0 — a non-interactive
  invocation is not an error.
- **Covered by:** `tests/cli__setup.rs::a_closed_stdin_prints_the_plan_and_the_yes_command`

### setup-02 Dry run

- **Flags:** `--dry-run` `--repo` `--host` `--json`
- **Setup:** an empty `--data-dir` and a fresh `git init` repo with real sources
- **Command:** `comemory setup --dry-run --repo /path/to/repo --host codex --json`
- **Expect:** all seven steps in the envelope, `git-hooks` and `index-code`
  `pending`, `applied: 0`, and no `comemory.db` created — planning is a read.
- **Covered by:** `tests/cli__setup.rs::dry_run_json_reports_a_plan_and_creates_no_database`

### setup-03 Apply everything

- **Flags:** `--yes` `--skip`
- **Setup:** an empty data dir and a fresh repo with four real `.rs` files
- **Command:** `comemory setup --yes --repo /path/to/repo --skip agent-host`
- **Expect:** `data-dir`, `git-hooks`, and `index-code` all `applied`; the
  four hook files exist; `comemory search-code gamma` then finds an indexed
  symbol. A second identical run applies nothing and reports `satisfied`.
- **Covered by:** `tests/cli_scenario_setup.rs::setup_yes_indexes_a_fresh_repo_then_search_code_finds_a_symbol`

### setup-04 One step only

- **Flags:** `--only`
- **Setup:** an empty data dir and a fresh repo
- **Command:** `comemory setup --yes --only git-hooks --repo /path/to/repo`
- **Expect:** `applied: 1`, `git-hooks` applied, every other pending step
  `skipped`, and no `comemory.db` — because `data-dir` was skipped too.
- **Covered by:** `tests/cli_scenario_setup.rs::only_runs_exactly_one_step_and_skips_the_others`

### setup-05 Unknown step id

- **Flags:** `--only`
- **Setup:** any data dir
- **Command:** `comemory setup --only bogus`
- **Expect:** exit 64, a message naming `bogus` and listing the known ids, and
  nothing probed — validation precedes detection.
- **Covered by:** `tests/cli__setup.rs::an_unknown_step_id_exits_64_and_names_it`

### setup-06 Conflicting intents

- **Flags:** `--yes` `--dry-run`
- **Setup:** any data dir
- **Command:** `comemory setup --yes --dry-run`
- **Expect:** clap refuses the pair with exit 2 rather than silently picking
  one.
- **Covered by:** `tests/cli__setup.rs::dry_run_and_yes_are_mutually_exclusive`

### setup-07 Nothing this machine can offer

- **Flags:** `--host` `--repo` `--dry-run`
- **Setup:** a directory that is not a git worktree, and no agent-host CLI
- **Command:** `comemory setup --dry-run --repo /tmp/not-a-repo --host codex`
- **Expect:** `git-hooks`, `index-code`, and `index-docs` all `unavailable`
  with reason `not a git repository`; `agent-host` unavailable naming `PATH`;
  `cloud-auth` unavailable naming `comemory auth login` — it is a report in
  every mode, never something setup applies, so it can never fail. Exit 0
  throughout: unavailable is never a failure.
- **Covered by:** `tests/cli__setup.rs::a_non_git_directory_reports_unavailable_repo_steps_and_exits_zero`,
  `tests/cli__setup.rs::cloud_auth_is_reported_not_applied_and_issues_no_request`

### setup-08 A hand-written hook is never clobbered

- **Flags:** `--yes`
- **Setup:** a repo carrying a `post-commit` hook comemory did not write
- **Command:** `comemory setup --yes --repo /path/to/repo`
- **Expect:** `git-hooks` reported `unavailable`, its reason naming
  `--force`, and the operator's hook file left byte-identical. Setup never
  passes `--force`.
- **Covered by:** `tests/cli__setup.rs::a_hand_written_hook_is_never_clobbered`

### setup-09 A failing step does not hide the others

- **Flags:** `--yes` `--only`
- **Setup:** a read-only data directory plus a writable repo (unix only)
- **Command:** `comemory setup --yes --only data-dir,git-hooks --repo /path/to/repo`
- **Expect:** `data-dir` `failed`, `git-hooks` still `applied` beside it, all
  seven steps present in the envelope, and exit 69 with stderr naming the
  failed id — the summary is rendered before the exit status is decided.
- **Covered by:** `tests/cli_scenario_setup.rs::a_failing_step_still_reports_every_step_and_exits_69`
