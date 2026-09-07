# `comemory auth`

Cloud workspace-key device login against the platform API
(`https://api.comemory.io` by default). Runs the RFC 8628 device flow
(`client_id=comemory-cli`), waits for console approval + workspace bind, then
`POST /v1/device/mint-workspace-key` with the device `access_token` as
`Authorization: Bearer …`, and writes a workspace-bound `cmk_` into
`$COMEMORY_DATA_DIR/auth.json` (mode `0600`). Nested: `login` / `status` /
`logout`. No sync / device-key mint in this slice.

**Runnable tests:** `tests/cli__auth.rs` (command; loopback fixture
`tests/common/device_auth_server.rs`), colocated `src/cloud/tests/*`

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

### auth-01 Login writes auth.json

- **Flags:** _(none)_
- **Setup:** loopback device auth fixture; `COMEMORY_API` / `--api-url` pointed at it
- **Command:** `comemory --json auth login --api-url http://127.0.0.1:<port>`
- **Expect:** exit 0; `auth.json` mode `0600` with `secret` matching `cmk_` + 64 hex;
  JSON includes `secret` once, `workspace_id`, `key_prefix`, `api_url`
- **Covered by:** `tests/cli__auth.rs::login_writes_auth_json_mode_0600_and_usable_secret`

### auth-02 Status authenticated after login

- **Flags:** _(none)_
- **Setup:** auth-01 credentials on disk
- **Command:** `comemory --json auth status`
- **Expect:** `{ "authenticated": true, "workspace_id", "workspace_name", … }`;
  no full `secret` field
- **Covered by:** `tests/cli__auth.rs::status_reports_authenticated_after_login`

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
