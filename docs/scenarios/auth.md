# `comemory auth`

Organization-scoped login against the platform API (`https://api.comemory.io`
by default). Runs the RFC 8628 device flow (`client_id=comemory-cli`), waits
for console approval — where the operator picks the organization — then
`POST /v1/device/mint-org-key` with the device `access_token` as
`Authorization: Bearer …`, and writes an org-scoped `cmk_` into
`$COMEMORY_DATA_DIR/auth.json` (schema v2, mode `0600`). `login` then runs the
first sync (pull, then push) before returning, best-effort: a platform outage
warns on stderr and still exits 0. Nested: `login` / `status` / `logout`.

**Runnable tests:** `tests/cli__auth.rs` (command; loopback fixture
`tests/common/device_auth_server.rs`), colocated `src/domains/sync/cloud/tests/*`

**HTTP:** none — cloud login is CLI-only (`transport: "cli-only"` in
`GET /api/v1/commands`; asserted by `tests/api__parity.rs`)

Global flags `--json` and `--data-dir` are accepted. See [globals.md](globals.md).

API base precedence: `--api-url` > `COMEMORY_API` > `https://api.comemory.io`.
`COMEMORY_API_KEY` overrides the on-disk secret for `status` (CI without
writing the file).

## Positionals

_None at the `auth` level._ Nested subcommand required: `login` | `status` |
`logout`.

## Flags

| Flag | Subcommand | Default | Effect |
| --- | --- | --- | --- |
| `--api-url` | `login`, `status` | `COMEMORY_API` / prod | Platform API base URL |

## Scenarios

### auth-01 Login writes a v2 auth.json carrying the org scope

- **Flags:** _(none)_
- **Setup:** loopback device auth fixture; `COMEMORY_API` / `--api-url` pointed at it
- **Command:** `comemory --json auth login --api-url http://127.0.0.1:<port>`
- **Expect:** exit 0; `auth.json` mode `0600`, `version: 2`, carrying
  `organization_id` / `organization_slug` / `workspace_id` and **not**
  `device_name` or `personal_workspace_id`; JSON includes `secret` once;
  `/v1/workspaces` is never called
- **Covered by:** `tests/cli__auth.rs::login_writes_v2_auth_file`

### auth-02 Status reports the bound organization

- **Flags:** _(none)_
- **Setup:** auth-01 credentials on disk
- **Command:** `comemory --json auth status`
- **Expect:** `{ "authenticated": true, "organization_id", "organization_name",
  "workspace_id", … }`; no full `secret` field
- **Covered by:** `tests/cli__auth.rs::status_reports_the_bound_organization`

### auth-06 Login runs the first sync before returning

- **Flags:** _(none)_
- **Setup:** loopback platform fixture serving both the device grant and the sync routes
- **Command:** `comemory --json auth login --api-url http://127.0.0.1:<port>`
- **Expect:** `initial_sync.ok = true` with `pulled` / `pushed` counts; the
  fixture records `GET /v1/sync/changes` before any `POST /v1/sync/import`
- **Covered by:** `tests/cli__auth.rs::login_runs_initial_sync_before_returning`

### auth-07 A sync outage does not fail the login

- **Flags:** _(none)_
- **Setup:** platform fixture answering 500 on every sync route
- **Command:** `comemory --json auth login --api-url http://127.0.0.1:<port>`
- **Expect:** exit 0; `auth.json` written; `initial_sync.ok = false` with an
  `error` string; a warning on stderr
- **Covered by:** `tests/cli__auth.rs::login_survives_a_sync_outage`

### auth-08 An unscoped mint leaves no credential behind

- **Flags:** _(none)_
- **Setup:** fixture minting an empty `workspaceId`
- **Command:** `comemory auth login --api-url http://127.0.0.1:<port>`
- **Expect:** non-zero exit naming the missing organization scope; no `auth.json`
- **Covered by:** `tests/cli__auth.rs::unscoped_mint_leaves_no_credential_on_disk`

### auth-09 Login and logout clear a stale allowlist

- **Flags:** _(none)_
- **Setup:** an `allowlist.json` left by a pre-org-scoping release
- **Command:** `comemory auth login …`, then `comemory auth logout`
- **Expect:** the file is gone after each; logout stays idempotent
- **Covered by:** `tests/cli__auth.rs::login_and_logout_clear_a_stale_allowlist`

### auth-10 A save pushes inline, with no daemon anywhere

- **Flags:** _(none)_
- **Setup:** platform fixture; a plain `comemory auth login`
- **Command:** `comemory save --repo acme/backend "<body>"`
- **Expect:** exactly one more `POST /v1/sync/import`, before the process
  exits, carrying the saved body.
- **Covered by:** `tests/cli__auth.rs::a_save_pushes_inline_without_any_daemon`

### auth-11 An unreachable platform cannot fail a save

- **Flags:** _(none)_
- **Setup:** logged in, then the stored `api_url` pointed at a closed port
- **Command:** `comemory save --repo acme/backend "saved while offline"`
- **Expect:** exit 0, and `comemory sync --action status` reports `pending ≥ 1`
- **Covered by:** `tests/cli__auth.rs::a_save_still_succeeds_when_the_platform_is_unreachable`

### auth-12 A plain login always reports `daemon.skipped: false`

- **Flags:** _(none)_
- **Command:** `comemory auth login`, then `comemory auth login --no-daemon`
- **Expect:** `login` is a `Required` command (#257): preflight already
  ensured a coordinator before it ran, so `daemon.skipped` is always
  `false` now — kept in the schema, not a live switch. `--no-daemon`, which
  opted out of the old opt-in install, is still refused as an unknown
  argument (exit 2), not ignored.
- **Covered by:** `tests/cli__auth.rs::login_reports_skipped_false_and_the_removed_no_daemon_flag_is_refused`

### auth-13 `--daemon` is a deprecated no-op

- **Flags:** `--daemon`
- **Command:** `comemory auth login --daemon`
- **Expect:** parses (hidden from `--help`) and warns on stderr that it is
  deprecated; the report is unchanged (`daemon.skipped: false`) — the
  coordinator is always ensured, not something this flag ever started.
- **Covered by:** `tests/cli__auth.rs::login_with_the_deprecated_daemon_flag_still_logs_in_and_warns`

### auth-03 Logout removes auth.json

- **Flags:** _(none)_
- **Setup:** auth-01 credentials on disk
- **Command:** `comemory auth logout` then `comemory --json auth status`
- **Expect:** logout exit 0; status `{ "authenticated": false }` and non-zero exit
- **Covered by:** `tests/cli__auth.rs::logout_removes_auth_json`

### auth-04 Wrong client / expired poll errors

- **Flags:** _(none)_
- **Setup:** fixture rejecting `client_id` or returning `expired_token`
- **Command:** `comemory auth login --api-url …`
- **Expect:** non-zero exit; stderr names the failure
- **Covered by:** `tests/cli__auth.rs::wrong_client_or_expired_poll_surfaces_error`

### auth-05 --api-url and COMEMORY_API override default

- **Flags:** `--api-url`
- **Setup:** loopback fixture; env / flag pointed at it (not prod)
- **Command:** `comemory auth login --api-url http://127.0.0.1:<port>`; and with
  `COMEMORY_API` set without the flag
- **Expect:** requests hit the override host; `auth.json` stores that `api_url`
- **Covered by:** `tests/cli__auth.rs::api_url_flag_and_comemory_api_override_default`
