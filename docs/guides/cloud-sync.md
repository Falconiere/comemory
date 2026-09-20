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
`$COMEMORY_DATA_DIR/auth.json` (mode `0600`), and **runs a full first sync
before it returns** — pulling every remote page, then pushing every local page:

```text
✓ logged in to Acme, Inc. (cmk_abcd)
  api https://api.comemory.io · credentials ~/.comemory/auth.json
  daemon: not installed (saves push inline; `comemory watch` for live pulls)
  synced: pulled 12 · pushed 3 · skip_repos=0
```

There is no second step, and nothing resident is installed. After login:

- **Push is inline.** `comemory save` and `comemory delete` send the outbox
  before they return, bounded by `[sync] push_on_save_timeout` (2s).
- **Pull is on demand:** `comemory sync`, or [`comemory watch`](#watch) to
  follow changes live.
- **The daemon is opt-in** (`comemory auth login --daemon`) for headless hosts
  that want pulls without either.

If the platform is unreachable at that moment the login still succeeds — the
key is already minted and useful — and says so on stderr:

```text
warning: first sync failed (…) — run `comemory sync` when the platform is reachable
```

`comemory auth status` reports the bound organization and whether the daemon
is running (it usually is not, and that is fine). `comemory auth logout` deletes the local credential and **stops**
the daemon (the unit stays installed for the next login). `COMEMORY_API_KEY`
overrides the stored secret without writing the file.

## `comemory watch` — live pulls, no daemon {#watch}

```bash
comemory watch            # follow until interrupted
comemory watch --once     # pull once the channel greets, then exit
```

`watch` mints a 60-second ticket (`POST /v1/ws/ticket`), opens the workspace
channel (`GET /v1/ws`), and pulls whenever a frame arrives — on connect
(`hello`) and after anyone writes (`change`).

The socket carries **nudges, never memories**: a frame names memory ids, ops
and content hashes, and the pull it triggers is the same cursored
`GET /v1/sync/changes` a manual sync runs, with your real credential. So a
missed frame costs latency rather than data, a duplicate frame costs one empty
pull, and a leaked ticket buys an id list rather than a corpus.

A refused or dropped socket is not an error — `watch` reconnects with jittered
backoff (1s → 30s) until you stop it.

## Sync daemon (opt-in)

| Command | Effect |
|---------|--------|
| `comemory sync daemon install` | Write LaunchAgent `io.comemory.sync` (macOS) or systemd `--user` `comemory-sync.service` (Linux) |
| `comemory sync daemon start` / `stop` | Start or stop; stop leaves the unit installed |
| `comemory sync daemon status` | installed / running |
| `comemory sync daemon uninstall` | Remove the unit |
| `comemory sync daemon run` | Foreground loop (what the supervisor runs) |

Default interval: `[sync] daemon_interval = "5s"` — each cycle `pull` then
`push`, with an occasional `verify` per `[sync] verify_every`. Windows is not
supported.

You probably do not need it. A save pushes itself and `comemory watch` covers
pulls; what is left for a daemon is a headless host that wants pulls without a
foreground process. Install it with `comemory auth login --daemon`, or
`comemory sync daemon install` at any time. (The old `auth login --no-daemon`
is gone — it opted out of an install that no longer happens.)

## What syncs

**Everything.** Every memory on the machine is offered to your organization,
which accepts or rejects it on membership alone — and so is the **code index**
of every repo you have run `comemory index-code` over, so the console's Code
graph and Repositories screens fill in from the first login with no further
step.

One filter runs on your machine first:

| Memory | Outcome | Counter |
|--------|---------|---------|
| Label matches `[sync] skip_repos` | Stays local | `skipped_config` |
| Body trips the secret scan | Withheld until `--allow-secret` | `blocked_secrets` |
| Anything else | Offered to the organization | `pushed` |

### The code index

What leaves the machine for a repo is a **snippet-free projection**: file
paths with their git blob OIDs, symbol names with their kinds, languages and
line ranges, the resolved `imports` edges, and the mined `co_changed` pairs.
**Never a line of source** — the workspace can draw the graph and list the
repo, but code search and the Context screen's snippets stay on the machine
that holds the checkout.

The unit is the file and the blob OID is its digest, so a push is a diff:
`comemory sync` reads the workspace's manifest for each repo and posts only
the files whose blob differs, the paths the workspace still holds and you no
longer index, and the co-change set when its cursor moved. A repeat push with
nothing changed costs one manifest read per repo. The push runs:

- at the end of `comemory auth login`'s first sync;
- on every `comemory sync` (`run` or `push`);
- at the tail of every `comemory index-code` on the CLI (the lazy background
  reindex included) — for that repo, only when its index moved;
- in the opt-in daemon's cycle, only for repos whose index moved.

`comemory sync --action status` lists one `code:` row per indexed repo with
its local head, the head last pushed, `moved_since_push`, and — on a row the
push refuses to offer — `withheld=worktree`, `withheld=missing_root` or
`withheld=no_checkout`.

A `repo_marker` row whose recorded root is not a repository is never
offered, however the row was minted:

| Row | Outcome | Counter |
|-----|---------|---------|
| Recorded root is a linked `git worktree` | Never pushed — a second checkout is not a repository, and its files already reach the workspace under the main checkout's label | `worktrees` |
| Recorded root is not on disk | Not pushed until the path is back (a removed worktree, a deleted clone, an unmounted volume) | `missing_root` |
| Recorded root is a directory git cannot open | Not pushed — `index-code` opens the root first thing, so nothing could refresh what would be sent | `missing_root` |

Both counters appear in the `code:` summary line only when they are non-zero.
Nothing is deleted: the local row stays, so `comemory repos` still lists it
and a remounted volume resumes pushing on the next run. A row you want gone
for good is a `DELETE /api/v1/repos/{name}` (disconnect) — and a repository
the console already shows from an earlier push is removed there, not here.

```toml
[sync]
code_index = false     # keep every code index on this machine
skip_repos = ["acme/secret-*"]   # withholds that repo's memories AND its index
```

> **This is wider than before, twice over.** An empty `repo` label used to keep
> a memory local forever. Because `repo` is filled in from the git repository
> of whatever directory `comemory save` ran in, that rule quietly meant *a note
> taken outside a worktree could never sync* — which is why it is gone. The
> per-repo GitHub App allowlist is gone too: a memory labelled with a repo the
> App was never installed on is now pushed.
>
> **Everything already on your machine is re-offered once** after the upgrade
> (migration `0017`), including memories saved long before this change. If you
> keep memories that should not reach your organization, set `skip_repos`
> **before** upgrading.

```toml
[sync]
skip_repos = ["acme/secret-*", "my-side-project"]
daemon_interval = "5s"
verify_every = "7d"
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

Works with the daemon stopped. There is no `--workspace`: the key names its
own, and it is the only one that key can reach. Switching organization means
`comemory auth login` again.

## Config (`config.toml`)

| Knob | Default | Behavior |
|------|---------|----------|
| `[sync] push_on_save` | `true` | Push the outbox inline after `save` / `delete` |
| `[sync] push_on_save_timeout` | `"2s"` | Budget for that inline push (must be > 0; use `push_on_save = false` to disable) |
| `[sync] daemon_interval` | `"5s"` | Sleep between daemon pull+push cycles |
| `[sync] verify_every` | `"7d"` | Hint / daemon interval for `sync --action verify` |
| `[sync] skip_repos` | `[]` | Repo-label globs to keep local |
| `[sync] after_save` | `false` | **Deprecated, ignored** — superseded by `push_on_save` |
| `[sync] pull_before_context_after` | `""` (off) | **Deprecated, ignored** — use `comemory watch` |
| `[embed] model` | `""` | Recorded for vector import compatibility |

Offline or 5xx → the outbox waits and the write still succeeds. Local verbs
stay green. `comemory sync --action status` reports `pending` — how many local
writes are still owed to the platform — which is the number to watch after a
spell offline.

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

`[sync] repos`, `[sync] allowlist_ttl`, `[sync] default_workspace`,
`[sync] after_save`, and `[sync] pull_before_context_after` still parse for
one release and are ignored, with a warning naming each. Remove them.

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
