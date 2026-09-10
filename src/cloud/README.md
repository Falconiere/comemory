# cloud/

**What belongs here:** the cloud-platform client for login — API base URL
resolution, RFC 8628 device code / token poll, `POST /v1/device/mint-org-key`,
and the key probe behind `comemory auth status`.
`src/cloud.rs` beside this folder re-exports the public surface and the
`comemory-cli` client id. HTTP shells out through
[`crate::fetch`](../fetch.rs) (curl/wget); no TLS stack in the crate, which
is why these calls do not go through `sync::client` (reqwest, built without
TLS features and therefore loopback-only).

**What does NOT belong here:** the `auth.json` schema — that is
[`sync::auth_file::AuthFile`](../sync/auth_file.rs), the one file both
`auth login` and `sync` read. Nor opening a browser, remote key revoke on
logout, or an `/api/v1` route — `auth` is CLI-only
(`serve::routes::meta::CLI_ONLY`). Nor the first sync that follows a
successful login: that is [`sync::initial`](../sync/initial.rs).

## Contents

| File | Primary item | Purpose |
| --- | --- | --- |
| `api_url.rs` | `resolve` | `--api-url` > `COMEMORY_API` > `https://api.comemory.io`; strip trailing slash |
| `device.rs` | `login` | Device code → poll (pending / slow_down) → `mint-org-key` with Bearer access_token; `org_status` |

Tests live beside the module under `tests/`. End-to-end CLI coverage is
`tests/cli__auth.rs` against a loopback device-auth fixture.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/cloud.rs`.
