# Agent skills and hooks

Comemory owns its Claude Code and Codex integration. Install both the skills and
lifecycle hooks with one command:

```sh
comemory install claude
comemory install codex
```

`claude-hooks` and `codex-hooks` are accepted aliases. `--dry-run` previews the
bundle location without modifying anything. `--json` returns the installation
report. `--config-dir DIR` targets an isolated host configuration directory;
otherwise the host honors `CLAUDE_CONFIG_DIR` / `CODEX_HOME` and its usual default.

The corresponding host CLI, Bash, Git, jq, and Python 3 must be available. The
binary embeds the integration and extracts a local marketplace beneath
`$COMEMORY_DATA_DIR/integrations/<version>` (default `~/.comemory`). It registers
`comemory@comemory` through the host's native plugin manager. The marketplace
path stays at `integrations/` across releases so native registration remains stable. No source checkout
or toolu installation is required. Re-running an identical version is safe;
modified bundle files are preserved and reported as a conflict. Restart the
host after installation. After upgrading comemory, rerun the install command.

SessionStart publishes `comemory/comemory.sh` and `comemory/skills.sh` under the
host configuration directory. The memory wrapper uses canonical Git scope so
worktrees share knowledge; `--repo NAME` selects an explicit scope. The agent
recalls targeted knowledge, verifies its work, compares existing memories,
saves useful corrections and discoveries, and records retrieval feedback.

Prompt and compaction hooks provide compact reminders. Stop runs local retrieval
maintenance at most once per UTC day in a detached process. These hooks do not
invoke models or upload transcripts.

The project-skills workflow preserves `.toolu/skills/` as its storage directory
for compatibility with existing procedures. It runs independently of toolu.
Configure it in the host's `comemory.json` or the repository's
`.claude/comemory.json` / `.codex/comemory.json` (project values override user
values):

```json
{
  "skills": { "comemory": true },
  "projectSkills": {
    "enabled": true,
    "staleAfterDays": 30,
    "archiveAfterDays": 90,
    "indexCap": 20
  }
}
```

Set `skills.comemory` to `false` to suppress integration context and project-skill
hooks, or disable the plugin in the host. Set `projectSkills.enabled` to `false`
to disable only the project-skill loop. The optional memory-count marker remains
compatible with toolu's statusline.

## Migrating from toolu

First install `comemory@comemory`, then disable or uninstall `comemory@toolu`
using the host's plugin manager. Keep only one enabled to avoid duplicate hooks.
Memory files, the database, and `.toolu/skills/` do not move. Copy any wanted
`projectSkills` settings from `toolu.config.json` into `comemory.json`.

This installer does not set up cloud capture, upload sessions, or install Git
indexing hooks. Git hooks remain available through `comemory install-hooks`.
Learning checkpoints are a separate follow-up to this ownership migration.
