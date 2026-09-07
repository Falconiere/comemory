# `comemory workspaces`

List org (and personal) workspaces visible to the device key minted by
`comemory auth login`. CLI-only — talks to `GET /v1/workspaces` on the
platform.

**Runnable tests:** `tests/cli__workspaces.rs`

**HTTP:** none (`transport: "cli-only"`)

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

_None beyond globals._

## Scenarios

### workspaces-01 Requires login

- **Flags:** _(none)_
- **Command:** `comemory workspaces`
- **Expect:** usage error `not logged in`.
- **Covered by:** `tests/cli__workspaces.rs::workspaces_without_login_is_usage_error`
