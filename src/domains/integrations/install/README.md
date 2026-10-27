# install/

**What belongs here:** the embedded half of `comemory install`.

| File | Primary item | Purpose |
| --- | --- | --- |
| `bundle.rs` | `extract` | Embeds the repository-root `integrations/agent/**` assets with `include_str!` and writes a versioned local marketplace atomically — staging directory, then rename. Re-extracting over an identical tree is a no-op; a file the operator has edited is refused with the edit left in place, and a symlinked destination or catalog is refused before anything is written. `write_mcp_manifest` writes the plugin root's `.mcp.json` (Claude Code and Codex's own launch manifest) the same way, but on every install regardless of the equality check — the manifest is deliberately outside `FILES` |
| `cursor.rs` | `install` | File-based Cursor install: skill dirs plus the `comemory` MCP server merge, owning only that key. No host CLI is invoked — `cursor-agent plugin` only manages git-URL marketplaces. Hook wiring is a follow-up |
| `dirs.rs` | `mirror_dir` | Shared compare-then-copy tree mirroring for file-based hosts: copy what is missing, keep byte-identical content, refuse differing files and symlinked paths |
| `hermes.rs` | `install` | File-based hermes install: skill dirs only, since hermes exposes no plugin CLI, hooks, or MCP |

Each file's tests live in `tests/` under the same name. `src/domains/integrations/install.rs` beside this folder holds the host
validation, the configuration-directory precedence (including muse's
`XDG_DATA_HOME` scoping), the per-version and
per-host `.installed-<host>` markers, the hook-dependency probes and the
registration dispatch: native plugin CLI for claude/codex (`plugin ...`) and
muse (`plugins install <path>`), file writers otherwise.
