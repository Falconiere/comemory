# Bundled agent integration

`agent/` is the standalone comemory plugin embedded by `src/domains/integrations/install/bundle.rs`.
The binary builds local Claude Code and Codex marketplace catalogs around it.
The plugin has no toolu runtime dependency; `.toolu/skills/` remains the project
procedure directory for compatibility.

The binary also writes `<bundle>/plugins/comemory/.mcp.json` on every install
(`bundle::write_mcp_manifest`) — the plugin-root manifest both Claude Code
and Codex read to register `comemory mcp` as a native MCP server, carrying
the installing binary's absolute path; it is a per-install artifact, not part
of the embedded bundle.

Two bundled skills close the learning loop: `agent-memory` (the MCP tools and
the recall/judge/save loop, with the wrapper as fallback) and
`memory-bootstrap` (indexes code and docs, then saves the decisions a fresh
repo cannot derive from itself, triggered by the SessionStart nudge when a
repo has zero memories). A `SessionEnd` hook (`hooks/session-end.sh`) launches
`comemory capture session --from-hook` detached so a session receipt is
captured without delaying the host's shutdown budget.

See [installation and migration](../docs/guides/agent-integration.md).
