# Cloud sync (auth / link / sync)

Share org-workspace memories across machines through `api.comemory.io`. Local
CLI stays the day-to-day engine; cloud sync is optional and **org-repo-only**
in v1.

Contract (platform + engine): the amended memory-sync design and the
comemory.io platform API (§ Slice 2). If a CLI flag name drifts while wire
seams settle, the **platform HTTP contract** at `api.comemory.io` is
authoritative.

## One-time login

```bash
comemory auth login --api-url https://api.comemory.io
# approve at the printed URL (console /device → “This device”)
comemory auth status
comemory workspaces
```

Credentials land in `$COMEMORY_DATA_DIR/auth.json` (mode `0600`).
`COMEMORY_API_KEY` overrides the stored secret. Logout deletes the file.

Device keys are scoped to `/v1/sync/*` and `GET /v1/workspaces`. Workspace-bound
`cmk_` keys (CI) write through `/v1/memories` and friends — not sync.

## What syncs

Only memories whose `repo` label matches the **server GitHub App allowlist**
for the target **org** workspace (lowercase `owner/name`, or a unique basename).
Personal / unbound repos stay local (`skipped_personal` / `skipped_not_in_org`
in `comemory sync --action status`). Sync into personal (`isPersonal`) cloud
workspaces is off in v1.

```bash
comemory sync                          # push then pull (default)
comemory sync --action push            # or pull / verify / status
comemory sync --workspace <org-ws-id>
comemory sync --allow-secret <id>      # explicit secret-scan override
```

`comemory link --workspace <id> --repo <label>` writes an optional
`[sync.repos]` cache override in `config.toml`. It is **not** the source of
truth — the Worker still rejects non-allowlisted imports with
`repo_not_allowed`.

## Auto-sync (`config.toml`)

| Knob | Default | Behavior |
|------|---------|----------|
| `[sync] after_save` | `true` | Best-effort push after save; never fails the save |
| `[sync] pull_before_context_after` | `"5m"` | Pull before `context` when the cursor is stale |
| `[sync] verify_every` | `"7d"` | Hint for periodic `sync --action verify` |
| `[sync] allowlist_ttl` | `"1h"` | Local allowlist cache TTL |
| `[embed] model` | `""` | Recorded for vector import compatibility |

Offline or 5xx → outbox waits. Local verbs stay green.

## Engine HTTP (loopback)

`comemory serve` exposes the engine half of the protocol (platform Worker
forwards here):

| Route | Role |
|-------|------|
| `GET /api/v1/sync/changes` | Append-only log page |
| `POST /api/v1/sync/import` | Apply a batch (rules 1–10; `repo_not_allowed` is Worker-side) |
| `GET /api/v1/sync/manifest` | 256-bucket integrity check |

See [The HTTP API](http-api.md) § Cloud sync. `auth` / `workspaces` / `link` /
`sync` themselves are **cli-only** — they talk to the platform, not loopback.

## Operator checklist (platform)

1. Create the **sync** GitHub App (not the OAuth sign-in App):
   `bun run github-app:create --manifest scripts/github-app-sync-manifest.json`
   in the comemory.io repo; rename written secrets to `GITHUB_SYNC_APP_*`.
2. Install the App on the GitHub org; console Accounts → Connect GitHub.
3. Org members: `auth login` → `sync --workspace <org-ws>`.
