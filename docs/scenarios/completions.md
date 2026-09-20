# `comemory completions`

Emit a shell completion script on stdout, or install and register per-user
completion scripts for Bash, Zsh, Fish, and PowerShell. No data-dir I/O.

**Runnable tests:** `tests/cli_completions.rs`

**HTTP:** `GET /api/v1/completions` — covered by `tests/serve__routes__meta.rs::v1_completions_returns_the_generated_script_envelope`

`--data-dir` is accepted and ignored. `--json` is ignored when emitting one
script and returns the installation report when used with `--install`.
See [globals.md](globals.md).

## Positionals

`<SHELL>` — `bash` \| `zsh` \| `fish` \| `powershell` \| `elvish`; required
unless `--install` is present.

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--install` | off | Install and register Bash, Zsh, Fish, and PowerShell completions for the current user |

## Scenarios

### completions-01 Each shell

- **Flags:** _(none)_
- **Command:** `comemory completions <shell>` for each of bash, zsh, fish,
  powershell, elvish
- **Expect:** non-empty stdout containing `comemory`.
- **Covered by:** `tests/cli_completions.rs`

### completions-02 Install every interactive shell

- **Flags:** `--install`
- **Command:** `comemory completions --install`
- **Expect:** generated scripts in the XDG/user completion locations; one
  managed registration block in Bash, Zsh, and PowerShell profiles; rerunning
  preserves unrelated profile text, permissions, and symlinks without
  duplicating blocks. An existing Bash login profile is reused.
- **Covered by:** `tests/cli_completions.rs`
