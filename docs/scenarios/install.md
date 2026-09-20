# `comemory install`

Install the bundled comemory skills and hooks through the native agent host's
plugin manager, and write `<bundle>/plugins/comemory/.mcp.json` naming this
binary so the host launches `comemory mcp` for the MCP transport. This is
separate from Git reindex hooks (`install-hooks`).

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

### install-02 `.mcp.json` manifest

- **Flags:** `--config-dir` `--json` (dry-run and a real install both shown)
- **Setup:** a temporary data and config directory; a real host CLI on `PATH`
  for the non-dry-run half.
- **Command:** `comemory install claude --dry-run --json --data-dir <tmp> --config-dir <tmp>`, then `comemory install claude --config-dir <tmp> --json`
- **Expect:** the dry-run report's `mcp_manifest` names
  `<bundle>/plugins/comemory/.mcp.json` and the file does not exist; the real
  install writes it with `command` equal to the installing binary's absolute
  path and `args` equal to `["mcp"]`; re-running install after the binary
  moved rewrites `command` without a `bundle differs` refusal.
- **Covered by:** `tests/cli_scenario_install.rs::install_writes_mcp_manifest` (dry-run half); `scripts/test-agent-install.sh` (real host install, both hosts); `domains::integrations::install::bundle::tests::write_mcp_manifest_writes_the_command_and_args_for_a_fresh_bundle`, `write_mcp_manifest_rewrites_for_a_moved_binary_without_a_bundle_differs_refusal`, `write_mcp_manifest_refuses_a_symlinked_target`, `write_mcp_manifest_refuses_a_symlinked_temp_file` (a symlink planted at the `.mcp.json.tmp` temp path, not just the final path, is refused rather than written through)
