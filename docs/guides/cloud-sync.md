# Cloud sync (auth / sync)

Share organization memories across machines through `api.comemory.io`. The
local CLI stays the day-to-day engine; cloud sync is optional, and what may
leave a machine is decided by **organization membership**.

Contract (platform + engine): the comemory.io platform API. If a CLI flag name
drifts while wire seams settle, the **platform HTTP contract** at
`api.comemory.io` is authoritative.

> **Requires a platform that accepts organization-scoped keys.** This client
> mints `POST /v1/device/mint-org-key` and sends no `X-Comemory-Workspace`
> header. Against an older deployment every sync call fails.

## Log in — and that is the whole setup

```bash
comemory auth login --api-url https://api.comemory.io
# approve at the printed URL; pick the organization there
```

Login mints a key scoped to the organization you approved, writes it to
`$COMEMORY_DATA_DIR/auth.json` (mode `0600`), and **runs the first sync before
it returns** — pulling what the organization already holds, then pushing what
this machine has:

```text
✓ logged in to Acme, Inc. (cmk_abcd)
  api https://api.comemory.io · credentials ~/.comemory/auth.json
  synced: pulled 12 · pushed 3
```

There is no second step. No workspace id to look up, no per-repo linking. From
here `[sync] after_save` keeps pushing as you save.

If the platform is unreachable at that moment the login still succeeds — the
key is already minted and useful — and says so on stderr:

```text
warning: first sync failed (…) — run `comemory sync` when the platform is reachable
```

`comemory auth status` reports the bound organization. `comemory auth logout`
deletes the local credential (no remote revoke). `COMEMORY_API_KEY` overrides
the stored secret without writing the file.

## What syncs

Every memory whose `repo` frontmatter is non-empty is offered to your
organization, which accepts or rejects it on membership alone.

Two filters run on your machine first:

| Memory | Outcome | Counter |
|--------|---------|---------|
| Empty `repo` label | Stays local, always | `skipped_personal` |
| Label matches `[sync] skip_repos` | Stays local | `skipped_config` |
| Anything else | Offered to the organization | `pushed` |

> **This is wider than before.** Under the old per-repo GitHub App allowlist, a
> memory labelled with a repo the App was not installed on stayed local. It is
> now pushed. If you keep memories labelled for repositories that should not
> reach your organization, set `skip_repos` before upgrading.

```toml
[sync]
skip_repos = ["acme/secret-*", "my-side-project"]
```

Patterns are globs matched against the trimmed, lowercased label, so
`Acme/Secret-Thing` and `acme/secret-thing` are the same thing. An invalid
glob fails at config load rather than silently withholding nothing.

## Manual sync

```bash
comemory sync                     # push then pull (default)
comemory sync --action push       # or pull / verify / status
comemory sync --allow-secret <id> # explicit secret-scan override
```

There is no `--workspace`: the key names its own, and it is the only one that
key can reach. Switching organization means `comemory auth login` again.

## Auto-sync (`config.toml`)

| Knob | Default | Behavior |
|------|---------|----------|
| `[sync] after_save` | `true` | Push after each save; a failure never fails the save |
| `[sync] pull_before_context_after` | `"5m"` | Pull before `context` when the cursor is stale |
| `[sync] verify_every` | `"7d"` | Hint for periodic `sync --action verify` |
| `[sync] skip_repos` | `[]` | Repo-label globs to keep local |
| `[embed] model` | `""` | Recorded for vector import compatibility |

Offline or 5xx → the outbox waits. Local verbs stay green.

## Upgrading from a device-key install

Three breaking changes, all resolved by logging in again:

1. **`auth.json` v1 is refused.** It holds an unbound device key the platform
   no longer accepts, so it is rejected up front rather than half-migrated:
   `credentials … predate organization scoping — run comemory auth login`.
   `comemory save` and `comemory context` stay silent about it; `comemory sync`
   and `comemory auth status` report it.
2. **`comemory link` and `comemory workspaces` are gone.** `link` only ever
   wrote `[sync.repos]`, which nothing read; with no allowlist there is nothing
   to cache. An org-scoped key reaches exactly one workspace, so there is no
   list to show — `comemory auth status` names it.
3. **`--device-name` and `--workspace` are gone**, along with `allowlist.json`
   (deleted at the next login or logout).

`[sync] repos`, `[sync] allowlist_ttl` and `[sync] default_workspace` still
parse for one release and are ignored, with a warning naming each. Remove them.

## Engine HTTP (loopback)

`comemory serve` exposes the engine half of the protocol (the platform Worker
forwards here):

| Route | Role |
|-------|------|
| `GET /api/v1/sync/changes` | Append-only log page |
| `POST /api/v1/sync/import` | Apply a batch (rules 1–10) |
| `GET /api/v1/sync/manifest` | 256-bucket integrity check |

See [The HTTP API](http-api.md) § Cloud sync. `auth` and `sync` themselves are
**cli-only** — they talk to the platform, not loopback.

## Operator checklist (platform)

1. Deploy a platform build that serves `POST /v1/device/mint-org-key` and
   derives the workspace from key scope.
2. Have organization members run `comemory auth login`. That is the whole
   client-side rollout.
