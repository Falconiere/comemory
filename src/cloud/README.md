# cloud/

**What belongs here:** the cloud-platform client for workspace-key device
login — API base URL resolution, RFC 8628 device code / token poll,
`POST /v1/device/mint-workspace-key`, and durable `auth.json` credentials
(mode `0600`). `src/cloud.rs` beside this folder re-exports the public
surface and the `comemory-cli` client id. HTTP shells out through
[`crate::fetch`](../fetch.rs) (curl/wget); no TLS stack in the crate.

**What does NOT belong here:** sync / `mint-device-key`, opening a browser,
remote key revoke on logout, or an `/api/v1` route — `auth` is CLI-only
(`serve::routes::meta::CLI_ONLY`).

## Contents

| File | Primary item | Purpose |
| --- | --- | --- |
| `api_url.rs` | `resolve` | `--api-url` > `COMEMORY_API` > `https://api.comemory.io`; strip trailing slash |
| `credentials.rs` | `Credentials` | `auth.json` load / atomic save (0600) / clear; `COMEMORY_API_KEY` override |
| `device.rs` | `login` | Device code → poll (pending / slow_down) → mint with Bearer access_token; `workspace_status` |

Tests live beside the module under `tests/`. End-to-end CLI coverage is
`tests/cli__auth.rs` against a loopback device-auth fixture.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/cloud.rs`.
