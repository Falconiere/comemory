# `comemory sync`

Push/pull memories against the organization the key from `comemory auth login`
is scoped to, through `api.comemory.io`. Organization membership is the
platform's gate; on this side one filter runs — a `[sync] skip_repos` glob
match (`skipped_config`) — plus client-side redaction. An empty `repo` label no
longer withholds anything: a memory saved outside a git worktree syncs like any
other. Distinct from git `memory-stores` sync (`[git] auto_sync`).

`run` and `push` also push the **code index** of every indexed repo — a
snippet-free projection (paths, blob OIDs, symbol names and line ranges, the
`imports` / `co_changed` edges), diffed by blob OID against the workspace's
manifest. `[sync] code_index = false` turns it off; `skip_repos` withholds a
repo's index along with its memories.

Most users never run this: `comemory auth login` performs the first full sync,
`save` and `delete` push inline afterwards, and [`comemory watch`](watch.md)
covers the pull direction. The user-level daemon is opt-in
(`comemory auth login --daemon`) for headless hosts.

**Runnable tests:** `tests/cli__sync.rs`

**HTTP:** none — platform `/v1/sync/*` forwarder (`transport: "cli-only"`).
Local engine also exposes `GET|POST /api/v1/sync/{changes,import,manifest}`
and the code-index pair `GET /api/v1/sync/code/manifest?repo=` /
`POST /api/v1/sync/code/import` for the host; those routes are covered by
`src/serve/routes/tests/sync.rs`, `src/serve/routes/tests/sync_code.rs` and
`src/api/sync/tests/`.

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--action` | `run` | `run` (push+pull), `push`/`push-only`, `pull`/`pull-only`, `verify`, `status` |
| `--allow-secret` | unset | Record a secret-scan override for one memory id before push |

`--action status` reports `pending` — how many local writes are still owed to
the platform — beside the two cursors, and one `code` row per indexed repo
(local head, last pushed head, `moved_since_push`).

Nested: `comemory sync daemon {install,uninstall,start,stop,status,run}` —
see [cloud-sync.md](../guides/cloud-sync.md).

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

### sync-06 The code index is pushed after the memories, as a diff

- **Flags:** `--action`
- **Setup:** an org credential against the loopback platform fixture; a real
  git repo indexed by `index-code`
- **Command:** `comemory sync --action push` (twice)
- **Expect:** the first run reads `GET /v1/sync/code/manifest` and posts one
  `POST /v1/sync/code/import` carrying every file; the second run reads the
  manifest and posts nothing. No request body carries source text.
- **Covered by:** `src/sync/tests/code.rs::ac12_first_push_sends_every_file_and_the_second_only_reads_the_manifest`,
  `src/sync/tests/code.rs::ac16_no_request_body_carries_source_text`

### sync-07 `[sync] code_index = false` and `skip_repos` withhold the index

- **Flags:** `--action`
- **Setup:** as sync-06, with `code_index = false` (or `skip_repos` matching
  the label)
- **Command:** `comemory sync --action push`
- **Expect:** no `/v1/sync/code/*` request at all; `skip_repos` counts the
  repo under `skipped_config`.
- **Covered by:** `src/sync/tests/code.rs::ac11_code_index_off_sends_nothing_even_with_an_index`,
  `src/sync/tests/code.rs::skip_repos_withholds_the_index_too`
