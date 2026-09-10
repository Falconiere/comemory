# `comemory sync`

Push/pull memories against the organization the key from `comemory auth login`
is scoped to, through `api.comemory.io`. Organization membership is the
platform's gate; on this side only two filters run — an empty `repo` label
(`skipped_personal`) and a `[sync] skip_repos` glob match (`skipped_config`) —
plus client-side redaction. Distinct from git `memory-stores` sync
(`[git] auto_sync`).

Most users never run this: `comemory auth login` performs the first sync, and
`[sync] after_save` keeps up from there.

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
| `--allow-secret` | unset | Record a secret-scan override for one memory id before push |

There is no `--workspace`: the org-scoped key names the only workspace it can
reach. Switching organization means running `comemory auth login` again.

## Scenarios

### sync-01 Status requires login

- **Flags:** `--action`
- **Command:** `comemory sync --action status`
- **Expect:** usage error `not logged in`.
- **Covered by:** `tests/cli__sync.rs::sync_without_login_is_usage_error`

### sync-02 Help lists the surviving flags

- **Flags:** `--allow-secret`
- **Command:** `comemory sync --help`
- **Expect:** help names `--action` and `--allow-secret`, and does **not**
  name `--workspace`.
- **Covered by:** `tests/cli__sync.rs::sync_action_help_lists_push_and_status`

### sync-03 A credential from before organization scoping is refused

- **Flags:** `--action`
- **Setup:** a hand-written v1 `auth.json` (no `version`, with `device_name`)
- **Command:** `comemory sync --action status`
- **Expect:** exit 64; stderr names `comemory auth login` and the cause.
- **Covered by:** `tests/cli__sync.rs::sync_rejects_a_credential_written_before_org_scoping`

### sync-04 `--workspace` is rejected

- **Flags:** _(none)_
- **Command:** `comemory sync --action push --workspace ws-somewhere`
- **Expect:** clap usage error naming `--workspace` as unexpected.
- **Covered by:** `tests/cli__sync.rs::sync_rejects_an_unknown_workspace_argument`

### sync-05 A config carrying every deprecated key still loads

- **Flags:** `--action`
- **Setup:** `config.toml` with `[sync.repos]`, `allowlist_ttl`, `default_workspace`
- **Command:** `comemory sync --action status`
- **Expect:** the config loads; the run reaches the login check rather than a
  config error.
- **Covered by:** `tests/cli__sync.rs::sync_loads_a_config_carrying_every_deprecated_key`
