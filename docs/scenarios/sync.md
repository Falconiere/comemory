# `comemory sync`

Push/pull memories against the organization the key from `comemory auth login`
is scoped to, through `api.comemory.io`. Every data request carries the
negotiated repository-policy protocol and revision. Memories sync only when
their label is a canonical approved GitHub repository, an administrator mapping,
or an unambiguous indexed checkout identity. Unlabelled and unresolved memories
remain local (`blocked_repo`), in addition to `skip_repos` and secret filtering.
Distinct from git `memory-stores` sync (`[git] auto_sync`).

`run` and `push` also push the **code index** of every indexed repo — a
snippet-free projection for each approved canonical repository (paths, blob OIDs, symbol names and line ranges, the
`imports` / `co_changed` edges), diffed by blob OID against the workspace's
manifest. `[sync] code_index = false` turns it off; `skip_repos` withholds a
repo's index along with its memories.

Most users never run this: `comemory auth login` performs the first full sync,
`save` and `delete` push inline afterwards, and [`comemory watch`](watch.md)
covers the pull direction. The user-level daemon is opt-in
(`comemory auth login --daemon`) for headless hosts.

**Runnable tests:** `tests/cli__sync.rs`, `tests/cli__sync_auto.rs`

**HTTP:** none — platform `/v1/sync/*` forwarder (`transport: "cli-only"`).
Local engine also exposes `GET|POST /api/v1/sync/{changes,import,manifest}`
and the code-index pair `GET /api/v1/sync/code/manifest?repo=` /
`POST /api/v1/sync/code/import` for the host; those routes are covered by
`src/serve/routes/tests/sync.rs`, `src/serve/routes/tests/sync_code.rs` and
`src/domains/sync/exchange/tests/`.

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--action` | `run` | `run` (push+pull), `push`/`push-only`, `pull`/`pull-only`, `verify`, `status`, `auto` |
| `--allow-secret` | unset | Record a secret-scan override for one memory id before push |
| `--path` | unset | With `--action auto` only: the checkout a git hook fired in, indexed first under its main worktree's label. Any other action refuses it (exit 64) |

`run` and `push` also re-index every **hooked** repo (one whose `.git/hooks`
carry comemory's hook) whose HEAD moved since its last index, before the code
push, and report it as `refresh` (`refreshed: N of M hooked repo(s)` on a
TTY).

`--action auto` is the unattended pass comemory's git hooks and the agent
`SessionStart` hook fire, from any cwd: index `--path` (when given), refresh
every stale hooked repo, then — only when logged in — pull, push, and push the
code of repos whose index moved. It needs no login, prints nothing without
`--json`, and exits 0 even when a network leg failed (the failure is in
`error`). Passes serialize on `sync.lock`; a trigger that finds a pass already
queued exits with `{"action":"auto","coalesced":true}`.

`--action status` reports `pending` — how many local writes are still owed to
the platform — beside the two cursors, and one `code` row per indexed repo
(local head, last pushed head, `moved_since_push`, plus `withheld=worktree` /
`withheld=missing_root` / `withheld=no_checkout` on a row the push never
offers).

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
- **Covered by:** `src/domains/sync/tests/code.rs::ac12_first_push_sends_every_file_and_the_second_only_reads_the_manifest`,
  `src/domains/sync/tests/code.rs::ac16_no_request_body_carries_source_text`

### sync-07 `[sync] code_index = false` and `skip_repos` withhold the index

- **Flags:** `--action`
- **Setup:** as sync-06, with `code_index = false` (or `skip_repos` matching
  the label)
- **Command:** `comemory sync --action push`
- **Expect:** no `/v1/sync/code/*` request at all; `skip_repos` counts the
  repo under `skipped_config`.
- **Covered by:** `src/domains/sync/tests/code.rs::ac11_code_index_off_sends_nothing_even_with_an_index`,
  `src/domains/sync/tests/code.rs::skip_repos_withholds_the_index_too`

### sync-04 An auto pass from anywhere refreshes a stale hooked repo

- **Flags:** `--action`
- **Setup:** a repo with comemory's hooks, registered by `install-hooks`; a
  commit made with hooks off (`git -c core.hooksPath=/dev/null commit`)
- **Command:** `comemory sync --action auto --json`, run from outside the repo, logged out
- **Expect:** `refresh.refreshed = 1`, `logged_in: false`, no `pull`; `comemory repos --json` shows the new HEAD, `status: "fresh"`.
- **Covered by:** `tests/cli__sync_auto.rs::an_auto_pass_from_an_unrelated_cwd_refreshes_a_stale_hooked_repo`

### sync-05 Extra triggers coalesce behind a running pass

- **Flags:** `--action`
- **Setup:** a pass holds `sync.lock`
- **Command:** three concurrent `comemory sync --action auto --json`
- **Expect:** two exit 0 at once with `{"action":"auto","coalesced":true}`; the third waits and runs a pass once the lock is released.
- **Covered by:** `tests/cli__sync_auto.rs::triggers_behind_a_running_pass_coalesce_into_one_queued_pass`

### sync-06 `--path` belongs to `--action auto`

- **Flags:** `--path`
- **Command:** `comemory sync --action push --path .`
- **Expect:** exit 64, `--path is only accepted with --action auto`.
- **Covered by:** `tests/cli__sync_auto.rs::path_is_refused_with_any_action_but_auto`

### sync-07 A hook in a linked worktree passes its checkout as `--path`

- **Flags:** `--path`
- **Setup:** a hooked repo and a `git worktree add` of it
- **Command:** a real commit in the worktree (the hook runs `comemory sync --action auto --path <worktree>`)
- **Expect:** the main repo's label moves to the worktree's HEAD; no repository is minted for the worktree directory.
- **Covered by:** `tests/cli__sync_auto.rs::a_commit_in_a_linked_worktree_indexes_under_the_main_label`
