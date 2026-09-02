# `comemory completions`

Emit a shell completion script on stdout. No data-dir I/O.

**Runnable tests:** `tests/cli_completions.rs`

**HTTP:** `GET /api/v1/completions` — covered by `tests/serve__routes__meta.rs::v1_completions_returns_the_generated_script_envelope`

Global flags `--json` and `--data-dir` are accepted and ignored.
See [globals.md](globals.md).

## Positionals

`<SHELL>` — `bash` \| `zsh` \| `fish` \| `powershell` \| `elvish`.

## Flags

_None besides globals._

## Scenarios

### completions-01 Each shell

- **Flags:** _(none)_
- **Command:** `comemory completions <shell>` for each of bash, zsh, fish,
  powershell, elvish
- **Expect:** non-empty stdout containing `comemory`.
- **Covered by:** `tests/cli_completions.rs`
