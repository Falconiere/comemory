# install/

**What belongs here:** the embedded half of `comemory install`.

| File | Primary item | Purpose |
| --- | --- | --- |
| `bundle.rs` | `extract` | Embeds the repository-root `integrations/agent/**` assets with `include_str!` and writes a versioned local marketplace atomically — staging directory, then rename. Re-extracting over an identical tree is a no-op; a file the operator has edited is refused with the edit left in place, and a symlinked destination or catalog is refused before anything is written. `write_mcp_manifest` writes the plugin root's `.mcp.json` (Claude Code and Codex's own launch manifest, and the binary path muse's `bin/mcp-server.sh` launcher reads) the same way, but on every install regardless of the equality check — the manifest is deliberately outside `FILES` |
| `cursor.rs` | `install` | File-based Cursor install: skill links into `~/.cursor/skills/` (its personal dir; `skills-cursor/` is reserved for built-ins) plus the `comemory` MCP server merge into `mcp.json`, owning only that key — other keys keep their order and bytes (`RawValue` + `IndexMap`), the replacement is created with the file's permissions, an empty file counts as `{}`, a symlinked `mcp.json` is merged into its target, the file is checked before any skill is linked, and a current entry is not rewritten. No host CLI is invoked — `cursor-agent plugin` only manages git-URL marketplaces. Hook wiring is a follow-up |
| `host.rs` | `Host` | The per-host table (`TABLE`): config-dir source and default (muse's `XDG_DATA_HOME` scoping included), registration `Mechanism`, `Presence` probe, plugin id, report wording and next steps. `install`, `setup` detection and the unavailable-step reason all read it instead of matching host names |
| `muse.rs` | `install` | Muse registration through `muse plugins install <bundle>`. After `muse plugins validate` passes on the new bundle, removes a previous comemory registration whose source lies in comemory's integrations dir (muse refuses one id from two sources, and the bundle path is versioned); refuses a `comemory` plugin from anywhere else. Leaves `muse plugins approve` to the operator |
| `skill_links.rs` | `link_skills` | File-based skill install for cursor-agent and hermes: each skill dir becomes a symlink into the versioned bundle, so wrappers reach the bundle's `lib/` and an upgrade is an atomic repoint. A real directory or a link comemory did not write is refused before anything changes |

Each file's tests live in `tests/` under the same name. `src/domains/integrations/install.rs` beside this folder holds the host
validation, the per-version and per-host `.installed-<host>` markers, the
hook-dependency probes (plugin hosts only) and the registration dispatch on
each `host::Host`'s `Mechanism`: native plugin CLI for claude/codex
(`plugin ...`) and muse (`plugins install <path>`), skill links (plus the
cursor `mcp.json` merge) otherwise.
