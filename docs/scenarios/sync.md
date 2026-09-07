# `comemory sync`

Push/pull memories against an org workspace through `api.comemory.io`
using the device key from `comemory auth login`. Applies the GitHub App
allowlist push filter (`skipped_personal` / `skipped_not_in_org` /
`skipped_ambiguous`) and client-side redaction. Distinct from git
`memory-stores` sync (`[git] auto_sync`).

**Runnable tests:** `tests/cli__sync.rs`

**HTTP:** none — platform `/v1/sync/*` forwarder (`transport: "cli-only"`).
Local engine also exposes `GET|POST /api/v1/sync/{changes,import,manifest}`
for the host; those routes are covered by `src/serve/routes/tests/sync.rs`
and `src/api/sync/tests/`.

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--action` | `run` | `run` (push+pull), `push`/`push-only`, `pull`/`pull-only`, `verify`, `status` |
| `--workspace` | config / personal | Target org workspace id |
| `--allow-secret` | unset | Record a secret-scan override for one memory id before push |

## Scenarios

### sync-01 Status requires login

- **Flags:** `--action`
- **Command:** `comemory sync --action status`
- **Expect:** usage error `not logged in`.
- **Covered by:** `tests/cli__sync.rs::sync_without_login_is_usage_error`

### sync-02 Help lists filter flags

- **Flags:** `--workspace` `--allow-secret`
- **Command:** `comemory sync --help`
- **Expect:** help names `--action`, `--workspace`, `--allow-secret`.
- **Covered by:** `tests/cli__sync.rs::sync_action_help_lists_push_and_status`
