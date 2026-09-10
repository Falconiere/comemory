# `comemory capture`

Client half of platform Slice 3: read a coding-tool transcript, redact it on
the machine, attest the rule set, and POST a **receipt** (never the body) to
`POST /v1/sessions`. Nested: `session` / `sources` / `install-hook`. Consent
is shown via `capture sources` — the CLI cannot grant it (console / workspace
key only). Distinct from document `comemory sources` (`sources.toml`).

**Runnable tests:** `tests/cli__capture.rs`, colocated
`src/capture/tests/*`, `src/sync/tests/redact.rs`

**HTTP:** none — platform `/v1/capture/*` + `/v1/sessions` (`transport: "cli-only"`).

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None at the `capture` level._ Nested subcommand required: `session` |
`sources` | `install-hook`.

## Flags

| Flag | Subcommand | Default | Effect |
| --- | --- | --- | --- |
| `--source` | `session` | `claude-code` | Capture source id |
| `--path` | `session` | unset | Explicit transcript JSONL |
| `--session-id` | `session` | unset | Look up under `~/.claude/projects` |
| `--from-hook` | `session` | false | Read SessionEnd JSON from stdin |
| `--dry-run` | `session` | false | Build receipt without POST |
| `--allow-secret` | `session` | false | POST even when findings are non-empty |
| `--settings` | `install-hook` | `.claude/settings.json` | Settings file to write |
| `--force` | `install-hook` | false | Replace non-comemory SessionEnd entries |

## Scenarios

### capture-01 Session dry-run against the real Claude Code fixture

- **Flags:** `--path`, `--dry-run`
- **Setup:** `tests/common/fixtures/claude-code-session.jsonl`; auth.json pointed at a loopback that is never contacted
- **Command:** `comemory --json capture session --path <fixture> --dry-run`
- **Expect:** exit 0; `posted=false`; `receipt.redaction.version=1`; turnCount/digest match the fixture
- **Covered by:** `tests/cli__capture.rs::session_dry_run_against_fixture`

### capture-02 Session posts Bearer + workspace header

- **Flags:** `--path`, `--allow-secret`
- **Setup:** `tests/common/capture_platform_server.rs` + auth.json
- **Command:** `comemory --json capture session --path <fixture>`
- **Expect:** exit 0; fixture records `POST /v1/sessions` with `Authorization` and `X-Comemory-Workspace`
- **Covered by:** `tests/cli__capture.rs::session_posts_to_loopback`

### capture-03 Sources lists consent

- **Flags:** _(none)_
- **Setup:** loopback capture platform + auth.json
- **Command:** `comemory --json capture sources`
- **Expect:** `{ "sources": […] }` including `claude-code`
- **Covered by:** `tests/cli__capture.rs::sources_lists_consent`

### capture-04 Install-hook writes SessionEnd

- **Flags:** `--settings`
- **Setup:** temp settings path
- **Command:** `comemory capture install-hook --settings <path>`
- **Expect:** file contains `SessionEnd` and `comemory-capture-session-end`
- **Covered by:** `tests/cli__capture.rs::install_hook_writes_session_end`

### capture-05 Unimplemented source is usage

- **Flags:** `--source`, `--path`, `--dry-run`
- **Setup:** fixture path
- **Command:** `comemory capture session --source cursor --path <fixture> --dry-run`
- **Expect:** non-zero; message names `not implemented`
- **Covered by:** `tests/cli__capture.rs::cursor_source_is_usage_error`

### capture-06 From-hook reads stdin

- **Flags:** `--from-hook`, `--dry-run`
- **Setup:** fixture path in SessionEnd JSON on stdin; auth present
- **Command:** `comemory --json capture session --from-hook --dry-run` ← stdin payload
- **Expect:** exit 0; receipt for that path
- **Covered by:** `tests/cli__capture.rs::from_hook_reads_stdin`

### capture-07 Force install-hook

- **Flags:** `--settings`, `--force`
- **Setup:** settings already containing a foreign SessionEnd entry
- **Command:** `comemory capture install-hook --settings <path> --force`
- **Expect:** exit 0; comemory marker present; foreign entry preserved unless it was a comemory duplicate
- **Covered by:** `tests/cli__capture.rs::install_hook_force_merges`

### capture-08 Session-id lookup (unit)

- **Flags:** `--session-id`
- **Setup:** colocated unit coverage of path resolution / fixture load
- **Command:** _(unit)_
- **Expect:** adapter loads metadata from the fixture when given a path; session-id errors when missing
- **Covered by:** `src/capture/tests/claude_code.rs::loads_fixture_session_metadata`
