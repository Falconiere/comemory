# `comemory distill`

Extract explicit `comemory save` invocations from a Claude Code JSONL
transcript, redact free text with a versioned attestation, and propose the
claims as platform candidate memories (`POST /v1/sessions/{id}/candidates`).
Requires a prior capture receipt for `--session-id` (platform Slice 3 / CLI
#121). Nothing is auto-accepted — a human reviews the queue on the platform.

**Runnable tests:** `tests/cli__distill.rs`,
`src/capture/tests/explicit_save.rs`, `src/capture/tests/candidates.rs`

**HTTP:** none — platform `/v1/sessions/{id}/candidates` (`transport: "cli-only"`).

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--session-id` | required | Platform session id from a capture receipt |
| `--transcript` | required | Path to a Claude Code JSONL transcript |
| `--dry-run` | off | Build and print the batch without POSTing (no login required) |
| `--api-url` | auth.json | Override the platform API base |

## Scenarios

### distill-01 Help lists the distill flags

- **Flags:** `--session-id`, `--transcript`, `--dry-run`, `--api-url`
- **Setup:** none
- **Command:** `comemory distill --help`
- **Expect:** success; stdout names all four flags
- **Covered by:** `tests/cli__distill.rs::distill_help_lists_required_flags`

### distill-02 Posting without login is a usage error

- **Flags:** `--session-id`, `--transcript`
- **Setup:** empty data dir (no `auth.json`)
- **Command:** `comemory distill --session-id sess-1 --transcript <fixture>`
- **Expect:** failure; stderr contains `not logged in`
- **Covered by:** `tests/cli__distill.rs::distill_without_login_is_usage_error`

### distill-03 Dry-run recovers the six real fixture saves

- **Flags:** `--session-id`, `--transcript`, `--dry-run`, `--json`
- **Setup:** platform fixture `tests/fixtures/claude-code-session-saves.jsonl`
- **Command:** `comemory distill --session-id sess-fixture --transcript <fixture> --dry-run --json`
- **Expect:** success; `extracted` is 6; first title and last kind match the platform reference extractor
- **Covered by:** `tests/cli__distill.rs::distill_dry_run_json_recovers_six_real_saves`
