# setup/

**What belongs here:** the terminal surface of `comemory setup`. Every
decision it appears to make is really made by
`domains::integrations::setup::plan` — these two files only select and render.

| File | Primary item | Purpose |
| --- | --- | --- |
| `wizard.rs` | `select` | The cliclack prompts. Holds no logic: it offers the pending steps and returns which were deselected. Cancellation (`ErrorKind::Interrupted`) becomes `Ok(None)`, and an all-satisfied plan short-circuits rather than building an empty multiselect |
| `render.rs` | `summary` | The non-interactive summary, written into an `impl Write` so it is snapshot-tested from `tests/cli__setup.rs` |

`src/cli/setup.rs` beside this folder holds the clap `Args`, the `Mode`
decision (`--dry-run` / `--yes` / TTY / piped), and the exit-code mapping.
