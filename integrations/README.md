# Bundled agent integration

`agent/` is the standalone comemory plugin embedded by `src/cli/install/bundle.rs`.
The binary builds local Claude Code and Codex marketplace catalogs around it.
The plugin has no toolu runtime dependency; `.toolu/skills/` remains the project
procedure directory for compatibility.

See [installation and migration](../docs/guides/agent-integration.md).
