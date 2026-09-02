# `comemory hooks`

Report (and toggle) the three git reindex hooks plus the config-backed
search→edit auto-reinforcement row.

**Runnable tests:** `tests/cli__hooks.rs`, `tests/cli_scenario_hooks.rs`

**HTTP:** `GET|POST /api/v1/hooks` — covered by `tests/serve_scenario_hooks.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--repo` | `.` | Git repo root the three hook files live in |
| `--enable` | unset | Install/enable one of `post-commit`, `post-merge`, `post-checkout`, `search-edit-reinforcement`. Conflicts with `--disable` |
| `--disable` | unset | Remove/disable one of the same four names |

## Scenarios

### hooks-01 Install then disable one

- **Flags:** `--repo` `--disable` `--json`
- **Setup:** a fresh git repo, then `install-hooks`
- **Command:**

```bash
comemory hooks --repo /path/to/repo --json
comemory hooks --repo /path/to/repo --disable post-commit --json
```

- **Expect:** after install, the three git hooks are `installed=true`. After
  disable, only that row flips; the other hook files are byte-identical.
- **Covered by:** `tests/cli__hooks.rs::ac35_fresh_repo_then_install_hooks_then_disable_one`,
  `tests/cli_scenario_hooks.rs`

### hooks-02 Search-edit reinforcement

- **Flags:** `--enable` `--disable`
- **Command:** `comemory hooks --disable search-edit-reinforcement --json`
- **Expect:** the config-backed row round-trips; git hook files are untouched.
- **Covered by:** `tests/cli__hooks.rs::ac36_search_edit_reinforcement_row_is_config_backed_and_round_trips`

### hooks-03 Enable

- **Flags:** `--enable`
- **Command:** `comemory hooks --repo /path/to/repo --enable post-checkout`
- **Expect:** that hook is installed again.
- **Covered by:** `tests/cli__hooks.rs` (toggle path; disable is the AC, enable is the inverse)
