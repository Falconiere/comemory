# `comemory install`

Install the bundled comemory skills — plus hooks and the `comemory mcp`
server where the host supports them — through the native agent host's plugin
manager or, for hosts without one, as skill links in its config dir. Every
install writes `<bundle>/plugins/comemory/.mcp.json` naming this binary. The
JSON report's `plugin`, `mcp_manifest` and `next_steps` describe what this
host actually got and what it still needs. This is separate from Git reindex
hooks (`install-hooks`).

**Runnable tests:** `tests/cli_scenario_install.rs`; native host installation,
upgrade, hook execution and worktree recall: `bash scripts/test-agent-install.sh`.

**HTTP:** none — CLI only.

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`<HOST>`: `claude`, `codex`, `muse`, `cursor-agent` or `hermes` (aliases
`claude-hooks` and `codex-hooks`; `muse-hooks`; `cursor` and `cursor-hooks`;
`hermes-hooks`). CLI-plugin hosts (`claude`, `codex`, `muse`) register
through the host binary; `cursor-agent` and `hermes` link skills into
`~/.cursor/skills` / `~/.hermes/skills` (plus the `comemory` entry in
`~/.cursor/mcp.json` for Cursor) and need no host CLI or hook dependencies.

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--dry-run` | off | Preview destinations without writes or host dependencies |
| `--config-dir` | host environment or home directory | Override host configuration location; for muse, the `muse` directory itself |

## Scenarios

### install-01 Isolated preview

- **Flags:** `--dry-run` `--config-dir` `--json` `--data-dir`
- **Setup:** an empty temporary directory.
- **Command:** `comemory install claude --dry-run --json --data-dir /tmp/memory-preview --config-dir /tmp/host-preview`
- **Expect:** JSON identifies `comemory@comemory`; neither destination is created.
- **Covered by:** `tests/cli_scenario_install.rs::installation_preview_does_not_write_or_require_a_host`

> **Not to be confused with the shell installer** (`install.sh` at the repo
> root, run via `curl … | sh` or `comemory upgrade`). That script — unrelated
> to this `comemory install <HOST>` command — now finishes only after the
> binary it just placed answers its own `sync daemon ensure` ready; a
> daemon that never comes up rolls the swap back (`rolled back to …`) or, on
> a first install with no previous binary to restore, keeps the new file and
> names the fix at its absolute path, `<dir>/comemory sync daemon ensure`.
> See [Upgrading comemory](../guides/upgrading.md) and the
> [upgrade scenarios](upgrade.md) for that behavior and its rollback.

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
- **Covered by:** `tests/cli_scenario_install.rs::install_writes_mcp_manifest` (dry-run half); `scripts/test-agent-install.sh` (real host install for the plugin-CLI hosts, plus a contained muse section: upgrade from a previous bundle version, refresh, foreign-source refusal, the manifest's MCP entry, and a hook test per hook id); `domains::integrations::install::bundle::tests::write_mcp_manifest_writes_the_command_and_args_for_a_fresh_bundle`, `write_mcp_manifest_rewrites_for_a_moved_binary_without_a_bundle_differs_refusal`, `write_mcp_manifest_refuses_a_symlinked_target`, `write_mcp_manifest_refuses_a_symlinked_temp_file` (a symlink planted at the `.mcp.json.tmp` temp path, not just the final path, is refused rather than written through)

### install-03 File-based hosts

- **Flags:** `--config-dir` `--data-dir`
- **Setup:** a temporary data and config directory; no host CLI is invoked.
  The skill wrappers' tools (bash, git, jq) must be on `PATH`; python3 is
  only probed for hook hosts.
- **Command:** `comemory install hermes --data-dir <tmp>/data --config-dir <tmp>/hermes`, run twice; the same for `cursor-agent`.
- **Expect:** each skill dir is a symlink into `<data>/integrations/<version>`,
  so the wrappers reach the bundle's `lib/`; cursor also gets
  `<config>/mcp.json` with the `comemory` entry (a symlinked `mcp.json` is
  merged into the file it links to); the rerun succeeds; the
  report makes no plugin or hook claims; the per-host `.installed-<host>`
  marker exists.
- **Covered by:** `tests/cli_scenario_install.rs::file_hosts_install_for_real_without_a_host_cli`; `domains::integrations::install::skill_links::tests::an_upgrade_repoints_links_from_a_previous_bundle_version`, `refuses_a_real_directory_or_a_foreign_link_and_changes_nothing`, `wrappers_find_their_bundle_libs_through_the_link`; `domains::integrations::install::cursor::tests::the_merge_keeps_key_order_and_foreign_bytes`, `the_merge_keeps_the_files_permissions`

### install-04 Muse configuration directory

- **Flags:** `--config-dir` `--dry-run` `--json`
- **Setup:** a temporary `HOME`.
- **Command:** `comemory install muse --dry-run --json --config-dir <tmp>/share`; then `XDG_DATA_HOME= comemory install muse --dry-run --json`.
- **Expect:** a `--config-dir` that does not end in `muse` exits 64 naming
  the expected `muse` directory; an empty or relative `XDG_DATA_HOME` is
  treated as unset, so `config_dir` is `$HOME/.local/share/muse`.
- **Covered by:** `tests/cli_scenario_install.rs::muse_config_dir_follows_the_xdg_rules`

### install-05 Muse MCP launcher

- **Flags:** none (behavior of the installed bundle)
- **Setup:** a plugin dir holding the bundled `bin/mcp-server.sh`, and a
  `.mcp.json` naming a stub binary.
- **Command:** `bash <plugin>/bin/mcp-server.sh`
- **Expect:** the launcher runs the binary `.mcp.json` names with `mcp`; with
  no readable `.mcp.json` it falls back to `comemory` on `PATH`.
- **Covered by:** `tests/cli_scenario_install.rs::muse_mcp_launcher_runs_the_installing_binary`
