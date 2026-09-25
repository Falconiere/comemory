# Cloud sync (auth / sync)

Share organization memories across machines through `api.comemory.io`. The
local CLI stays the day-to-day engine; cloud sync is optional, and what may
leave a machine is decided by the organization's approved repository policy.

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
  synced: pulled 12 · pushed 3 · skip_repos=0 · blocked_repo=0
```

There is no second step, and nothing resident is installed. After login:

- **Push is inline.** `comemory save` and `comemory delete` send the outbox
  before they return, bounded by `[sync] push_on_save_timeout` (2s).
- **Hooked repos sync themselves.** Every git operation that moves HEAD in a
  repo with comemory's hooks, and every agent session start, runs one
  [`sync --action auto`](#hooks) pass from wherever it happens: stale hooked
  repos are re-indexed, then pulled and pushed.
- **Pull is otherwise on demand:** `comemory sync`, or
  [`comemory watch`](#watch) to follow changes live.
- **The daemon is opt-in** (`comemory auth login --daemon`) for headless hosts
  that want pulls without any of those.

If the platform is unreachable at that moment the login still succeeds — the
key is already minted and useful — and says so on stderr:

```text
warning: first sync failed (…) — run `comemory sync` when the platform is reachable
```

`comemory auth status` reports the bound organization and whether the daemon
is running (it usually is not, and that is fine). `comemory auth logout` deletes the local credential and **stops**
the daemon (the unit stays installed for the next login). `COMEMORY_API_KEY`
overrides the stored secret without writing the file.

## Automatic sync from git hooks {#hooks}

Install comemory's hooks once per repository — that is the only per-repo step,
and you never have to `cd` into the repo again to keep it synced:

```bash
comemory install-hooks --repo ~/src/api     # or `comemory setup` inside it
```

That writes `post-commit`, `post-merge`, `post-checkout` and `post-rewrite`
into the repo's hooks directory and immediately runs the first pass, so the
repo is indexed (and, logged in, pushed) right away rather than at its next
commit. From then on, one pass runs whenever any of these fires:

| Trigger | Runs from | Pass |
|---|---|---|
| A commit, merge, pull, checkout, rebase or amend in any hooked repo | that repo's hook | `comemory sync --action auto --path <checkout>` |
| An agent session start (comemory plugin for Claude Code / Codex) | the session, any cwd | `comemory sync --action auto` |
| `comemory sync` (`run` / `push`), the opt-in daemon cycle | anywhere | the same refresh, before the code push |

A pass:

1. indexes the checkout that fired it (a linked worktree files under its main
   repository's label, as always);
2. re-indexes every other repo whose hooks are comemory's and whose HEAD
   moved since its last index — a rebase with hooks off, a commit from a GUI
   client, a repo you have not touched in a week;
3. when logged in: pulls, pushes memories, and pushes the code index of every
   repo whose index moved.

Logged out, steps 1–2 still run and nothing leaves the machine. A network
failure in step 3 is reported, not fatal: the outbox waits, as it does after
an offline `save`. Archived repos, repos without comemory's hooks, and
registered roots that are gone, unopenable or linked worktrees are never
re-indexed by the sweep (the same rows a code push withholds).
`COMEMORY_INDEXING_AUTO_REINDEX` does not gate it — installing the hooks is
the opt-in.

**Coalescing.** Passes run one at a time (`$COMEMORY_DATA_DIR/sync.lock`). A
rebase can fire a dozen hooks in a second; each trigger that finds a pass
already queued behind the running one exits at once
(`{"action":"auto","coalesced":true}` under `--json`), because that queued pass
has not started and will see its change. A trigger for a checkout no later
sweep would cover — a never-registered repo, or a linked worktree — waits its
own turn instead, so nothing is lost.

**Finding the binary.** The hook looks for `comemory` on `PATH`, then in
`~/.cargo/bin`, `/opt/homebrew/bin`, `/usr/local/bin` and `~/.local/bin` —
GUI git clients and IDEs often run hooks with none of those on `PATH`. It
exits 0 silently when none is found; a hook never fails a git operation.
Hooks written by an older release keep working (they run `index-code`
directly) until `comemory install-hooks` or `comemory setup` rewrites them.

## `comemory watch` — live pulls, no daemon {#watch}

```bash
comemory watch            # follow until interrupted
comemory watch --once     # pull once the channel greets, then exit
```

`watch` mints a 60-second ticket (`POST /v1/ws/ticket`), opens the workspace
channel (`GET /v1/ws`), and pulls whenever a frame arrives — on connect
(`hello`) and after anyone writes (`change`).

The socket carries **nudges, never memories**: a frame names memory ids, ops
and content hashes, and the pull it triggers is the same [exchange](#exchange)
a manual sync runs — pull direction only, pass after pass until the upstream
is drained — with your real credential. So a
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

Default interval: `[sync] daemon_interval = "5s"` — each cycle is the
[`--action auto`](#hooks) pass (refresh stale hooked repos, `pull`, `push`,
push moved code) under the same lock, with an occasional `verify` per
`[sync] verify_every`. Windows is not
supported.

You probably do not need it. A save pushes itself and `comemory watch` covers
pulls; what is left for a daemon is a headless host that wants pulls without a
foreground process. Install it with `comemory auth login --daemon`, or
`comemory sync daemon install` at any time. (The old `auth login --no-daemon`
is gone — it opted out of an install that no longer happens.)

## What syncs

The CLI fetches the workspace's authoritative repository policy before every
sync leg. A memory leaves the machine only when its local label is already a
canonical approved GitHub `owner/name`, an administrator has confirmed a label
mapping, or the label belongs to an indexed checkout whose current `origin`
unambiguously resolves to the approved repository. A free-text label alone is
not proof. The code index follows the checkout's current GitHub origin and is
sent under that canonical identity without rewriting the local label.

One filter runs on your machine first:

| Memory | Outcome | Counter |
|--------|---------|---------|
| Label matches `[sync] skip_repos` | Stays local | `skipped_config` |
| Missing, unsupported, ambiguous or unapproved repository identity | Stays local | `blocked_repo` |
| Body trips the secret scan | Withheld until `--allow-secret` | `blocked_secrets` |
| Approved canonical identity | Offered with an id-to-repository binding | `pushed` |

If policy cannot be fetched or validated, explicit sync fails closed and sends
nothing. Local `save`, `delete`, and `index-code` operations still succeed
offline because their automatic push is best effort. A policy or checkout
identity change resets memory reconciliation and code cursors so previously
withheld entries are reconsidered without requiring another save.

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

#### Engine to engine: whole generations

Between two comemory engines the same projection travels as one immutable
unit — a **generation** — rather than as a file diff. A generation is that
machine's complete answer for a repository at one revision, identified by the
digest of its own contents, and it becomes visible on the receiving machine at
a single instant: never some files at the old head and some at the new.

What that buys you:

- **A machine with no checkout is still useful.** It lists the repository
  (`comemory repos` shows `status: "shared"` and a `shared_head`) and answers
  the code graph for it. `search-code` returns nothing, because a generation
  carries no source — that is the same rule as above, not a new one.
- **Connecting a real checkout later adds to it.** The repository stays ONE
  row, now carrying both revisions, and local snippets start answering search.
  An edge both sides know is shown once, at the local weight.
- **Importing never touches what you indexed.** Your `code_symbols`, vectors
  and cursors are yours; a peer's generation lives in its own tables.
- **Only what this machine built is ever offered.** A projection you pulled is
  never pushed back.
- **A deleted file needs no announcement.** The next generation does not name
  it, so it leaves the peer exactly once.
- **An unmounted checkout deletes nothing.** The repository is withheld from
  the push with its reason readable (`missing_root`), its index untouched, and
  reconnecting the volume resumes pushing.

If two machines build a generation from the same parent, the first accepted
wins and the second is answered `rejected_stale`: it replans against the new
one rather than merging two heads into a tree neither machine has. One stale
generation never fails the batch it travelled in, and a retry reads the same
answer back. The full contract is
[code generation replication](../designs/2026-09-22-code-generation-replication.md).

- at the end of `comemory auth login`'s first sync;
- on every `comemory sync` (`run` or `push`);
- at the tail of every `comemory index-code` on the CLI (the lazy background
  reindex included) — for that repo, only when its index moved;
- on every [`--action auto`](#hooks) pass a hook fires, and in the opt-in
  daemon's cycle — only for repos whose index moved.

`comemory sync --action status` lists one `code:` row per indexed repo with
its local head, the head last pushed, `moved_since_push`, and — on a row the
push refuses to offer — `withheld=worktree`, `withheld=missing_root` or
`withheld=no_checkout`.

A `repo_marker` row whose recorded root is not a repository, has no supported
GitHub origin, or resolves outside the approved set is never offered:

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

> Repository labels remain local metadata. The CLI does not rewrite historical
> frontmatter when an administrator confirms a mapping; it attaches the
> canonical repository only to the managed import request.

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
comemory sync --action auto       # the hook pass: refresh hooked repos, then sync
comemory sync --allow-secret <id> # explicit secret-scan override
```

Works with the daemon stopped. There is no `--workspace`: the key names its
own, and it is the only one that key can reach. Switching organization means
`comemory auth login` again.

A run keeps going until the upstream is drained — there is no per-run cap —
and prints (or, with `--json`, returns) one `exchange` leg: `pushed`,
`pulled`, `held`, `settled`, `rejected`, `end`, `more`, `rebootstrapped` and
`network`. On a `replica-v1` key that leg replaces the `push`, `pull` and
`code` legs, which are then `null`. A run that ended on the network still
writes its report, then exits `69` with the recorded error.

## Config (`config.toml`)

| Knob | Default | Behavior |
|------|---------|----------|
| `[sync] push_on_save` | `true` | Push the outbox inline after `save` / `delete` |
| `[sync] push_on_save_timeout` | `"2s"` | Budget for that inline push (must be > 0; use `push_on_save = false` to disable) |
| `[sync] daemon_interval` | `"5s"` | Sleep between daemon pull+push cycles |
| `[sync] request_timeout` | `"30s"` | Budget for every sync request except the inline push |
| `[sync] pass_budget` | `"30s"` | An unattended pass starts no new batch after this and reports `more` |
| `[sync] max_request_bytes` | `4194304` | Largest push request; one operation over it crosses in staged parts |
| `[sync] verify_every` | `"7d"` | Hint / daemon interval for `sync --action verify` |
| `[sync] skip_repos` | `[]` | Repo-label globs to keep local |
| `[sync] after_save` | `false` | **Deprecated, ignored** — superseded by `push_on_save` |
| `[sync] pull_before_context_after` | `""` (off) | **Deprecated, ignored** — use `comemory watch` |
| `[embed] model` | `""` | Recorded for vector import compatibility |

Offline or 5xx → the outbox waits and the write still succeeds. Local verbs
stay green. `comemory sync --action status` reports `pending` — how many local
writes are still owed to the platform — and, in its `exchange` block, why
each owed or held change is waiting ([below](#exchange)); those are the
numbers to watch after a spell offline.

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

## Documents a repository shares

A document is replicated under a name that does not depend on where the file
sits: the digest of the canonical repository and the document's
repository-relative path. Two machines that indexed the same file from two
checkouts therefore agree on it, and no path of yours crosses the wire — what is
sent is the extracted text, its passages and its links, never the original file.

Sharing is off unless three things are true: the source was registered with
`--repo`, a policy load has approved that repository, and the repository has an
indexed root on this machine (`comemory index-code`). `comemory sources` names
which one is missing rather than leaving you to guess, and a document that is
not shared is still indexed and searchable here.

A revision whose text, title, headings or links match a redaction rule is
withheld **whole** — shared entirely or not at all — and `comemory sources`
reports the rule that stopped it.

A machine that pulled a document can search it with no registration and no file:
`comemory search --only document --json` marks those hits with `shared_from` and
`revision`, because there is nothing local to open. When both machines hold the
same document, search returns it once, from the local copy. Revoking a
repository's approval stops its pulled revisions answering immediately and
reapproving resumes them, with no re-indexing either way. The full contract is
[document revision replication](../designs/2026-09-23-document-revision-replication.md).

## Feedback and activity a repository shares

Verdicts (`comemory feedback`, the HTTP and MCP feedback routes) and the
activity feed replicate as events (`replica-v1` kinds `feedback_event` and
`activity_event`), never as counters: each machine adds one
contribution per event it receives, so replaying, relaying or echoing an event
back changes nothing, while two separate verdicts on the same result both
count. Every event carries the device that recorded it, its original time, the
surface and declared caller, and its provenance.

What is shared, exactly:

| History | Shared | Stays local |
| --- | --- | --- |
| Verdicts | `manual`, `implicit` and search→edit (`auto_search_edit`) verdicts on a memory or code symbol whose repository is approved | co-activation rewards (every indexing machine mints its own), unscoped targets, verdicts recorded before this release on code symbols |
| Activity | `save`, `update`, `delete`, `restore`, `search`, `context`, `find`, `search-code`, `feedback` and `index-code` runs scoped to an approved repository | unscoped runs, `sync.import`, every other command |
| Activity summaries | the counts, ids and kind fields each command lists, and the query text with machine paths replaced by `<path>` | memory titles, local query ids, result lists, and any query or caller label that matches a secret rule (the event says `query_withheld: true`) |
| Counters | nothing — each machine derives its own from the events | the contribution of events that expired before they were shared |
| Query history | nothing: a verdict cites `<device>:<query id>`, which never matches a local query | `retrieval_log`, candidate observations and judgments, bandit state |

An imported verdict never enters this machine's golden harvest, never marks a
query successful for `comemory mine`, and is not counted by
`comemory recall-status`. An imported activity run appears in
`GET /api/v1/activity` with its origin `device`; this machine's own runs have
`device: null`. `activity.enabled = false` records nothing to share, and
`activity.summaries = false` shares runs without their summary; on a receiving
machine the same two settings decide whether an imported run is shown and with
what. `comemory gc` expires the shared copy of an event when its retention
window passes, and purging a memory erases the shared copies of the verdicts on
it; a peer that offers either again is answered, not re-counted. Events
travel with the [exchange](#exchange): each sync first shares any runs
recorded since the last one, then queues every event journalled here since
the last pass, so a verdict reaches the workspace on the next sync like a
memory edit does. The full contract is
[feedback and activity replication](../designs/2026-09-24-feedback-activity-replication.md).

## What replicates, and what does not

Every write that changes a memory a peer can observe produces exactly one
replication operation — `comemory save`, an edit, a restore, a delete, and
`comemory prune --apply`, whichever surface made it (CLI, HTTP or MCP).

Three commands deliberately produce none. `refresh_refs` re-pins a code
anchor, `comemory doctor --reembed` recomputes vectors, and `comemory rebuild`
replays the mirror from markdown: each changes local state, none changes the
memory a peer holds, so journalling them would hand peers positions for
changes they cannot observe.

A save never depends on the network. Logged out, offline, or with
`COMEMORY_API` pointing at nothing, `comemory save` still succeeds and the
operation is queued for whenever the push works. A real persistence failure —
a read-only data directory, say — fails loudly instead.

## Embeddings across machines

An imported embedding is only usable when it came from the model this engine
queries with, at the dimension its vector table was built for. One that is
not is refused, the memory's text is stored regardless, and the id is recorded
as needing an embedding. `comemory doctor` reports the backlog as a warning
with the route that drains it, and `GET /api/v1/sync/replica/manifest` carries
the count.

So two machines on different embedders replicate every memory correctly, and
the one importing answers semantic search short until it re-embeds. Run
`comemory doctor --reembed` (or `POST /api/v1/doctor/reembed`) to fix it.

## An unpushed edit is not overwritten

A memory edited on this machine and not yet pushed exists in exactly one
place: the operation queued for it. A pull carrying a peer's version of that
same memory is never applied over the local edit: the old protocol answers it
`rejected_stale`, and a `replica-v1` pull holds it `pending_local` and comes
back for it once the local edit has gone out. The two changes then order
properly.

## An interrupted write

A memory write places its markdown and then mirrors it into the database.
A process killed in between is finished by the next command that opens the
data directory — the memory is mirrored, its operation is journalled, and the
upload it owes is queued. Running the same command twice changes nothing
further. A `--read-only` session writes nothing and leaves the recovery for
the next writable open.

## The exchange: how a sync drains {#exchange}

Every sync — `comemory sync`, a hook's `--action auto`, a daemon cycle, a
`comemory watch` nudge, the inline push after a save — runs the same
**exchange** for the key `auth.json` names, `(api_url, workspace)`. It pulls
first, then alternates bounded pull pages (500 entries) and push batches
(500 operations, `max_request_bytes`) until the pull reaches the head the
upstream reported when the pass began and nothing eligible is left to send.

### Which protocol

The first request of a pass asks the upstream for its replica manifest.

| The upstream | The key speaks | `coverage` |
| --- | --- | --- |
| serves `replica-v1` and has finished seeding its journal | `replica-v1` | `full` |
| serves `replica-v1` but is still seeding | the old protocol, this run | `partial`, `coverage_reason: "upstream_not_ready"` |
| has no replica routes | the old protocol | `partial`, `coverage_reason: "replica_unsupported"` |

The selection is remembered per key and only ever moves from the old
protocol to `replica-v1` — once, the first time the upstream is ready. A
revoked credential or a corrupt answer never changes it: a key already on
`replica-v1` is never silently downgraded. On that first upgrade, memory
changes the old protocol may already have delivered are held `upgrade` until
the pull has read up to the upstream's head at the switch; the pull then
recognizes each one the upstream already holds by its exact content and
settles it `already_upstream` instead of sending it twice.

The old protocol keeps its request shapes. What the exchange adds to it: no
2,000-entry cap per run, and a pull that never moves its cursor past an entry
the import did not apply — an `invalid` entry stalls it (status names the
entry), while a teammate's secret-bearing or unapproved entry is passed and
counted as a held position.

### How a pass ends

| `end` | When | `more` |
| --- | --- | --- |
| `caught_up` | the pull reached the head the pass began with, nothing eligible left | `true` only if a page reported a higher head |
| `budget` | an unattended pass spent `pass_budget` | `true` |
| `stalled` | the pull stopped before an entry it cannot apply (nothing eligible left to send) | `false` |
| `no_progress` | a full pull-and-push round moved nothing | `false` |
| `network` | a request failed after its retries within the pass | `false` |

`more: true` runs the next pass at once — a hook's pass keeps going, a daemon
cycle skips its sleep, `watch` pulls again without a nudge — so one nudge
drains any backlog. The inline push after a save never waits for a pass: when
one holds the sync lock it skips, and the running pass sends the new change
on its next batch.

### What status shows

`comemory sync --action status --json` carries an `exchange` block beside the
legacy cursors:

- `protocol`, `coverage`, `coverage_reason` — as above.
- `network` — `ok`, `backoff` (with `retry_at` and `consecutive_failures`),
  `auth_suspended` or `protocol_error`, and `last_error`.
- `stream_epoch`, `applied_sequence`, `upstream_head` — the pull cursor and
  the head the last completed pass saw.
- `caught_up` — true only when the cursor equals that head, no replay is in
  progress, nothing eligible is owed, nothing is stalled and the network is
  `ok`. A different key or a replaced stream never inherits it.
- `outbox` — `pending` (never attempted), `retryable` (attempted without an
  answer), `rejected`, and `held` by reason: `policy` (repository not
  approved), `secret` (a secret rule without an override —
  `comemory sync --allow-secret <id>` records one), `skip_repos`,
  `workspace` (made under another key — see below), `incompatible` (a kind
  the upstream does not read), `order` (behind a held change to the same
  entity, or a restore whose deletion the upstream has not placed yet) and
  `upgrade`.
- `pull` — held positions by reason (`policy`, `pending_local`,
  `server_withheld`, `secret`, `id_collision`), `stalled_at` and
  `stall_reason`.

A held change is never sent and never marked delivered; every pass decides
its hold again, so it goes out on its own once the cause is gone. Held pull
positions are revisited the same way: an approval restored, a newer policy
revision, or a local edit that has gone out rewinds the cursor to the
earliest one, and the replay can never put an older revision over a newer.

### Failures

| The upstream answers | State | Next request |
| --- | --- | --- |
| a timeout, a refused connection, `5xx` | `backoff` | after a full-jitter wait up to `min(2^n s, 5 min)`; a person's `comemory sync` retries at once |
| `429` with `Retry-After` (seconds or a date, capped at an hour) | `backoff` | not before `Retry-After` — for every run, a person's included |
| `401` / `403` | `auth_suspended` | only once `auth.json` holds a different credential |
| `409 sync_policy_changed` | — | the policy is reloaded and the pass runs once more, then backs off |
| `409 epoch_mismatch` / `cursor_ahead`, or the entry at the cursor changed | rebootstrap | at once |

Within a pass a failed request is retried at most twice. `auth_suspended`
stops network work only: the daemon keeps running, saves keep journalling,
status keeps counting what is owed, and writing a new credential resumes it
without a restart.

### Verify, restore and rebootstrap

`comemory sync --action verify` on a `replica-v1` key first drains to the
head, then compares, for every kind the upstream reads, its 256 bucket
digests with the same buckets over what this key last exchanged. A kind that
differs is replayed from the start for that kind alone, then compared again;
what still differs is reported (`kinds[].differing_buckets`,
`repaired: false`) beside `held_positions`, never looped on.

An upstream restored from a backup (its head below the cursor) or replaced
by a new stream (a new epoch) makes the next pass **rebootstrap**: the cursor
restarts on the stream the upstream now serves, held positions and synced
positions are dropped, and the stream is replayed keeping only each entity's
last entry, so an older revision never lands over a newer one. Pending local
changes stay pending and are sent; nothing local is deleted. A change the
upstream acknowledged and then lost in the restore is reported by verify, not
re-sent. The run that finishes the replay reports `rebootstrapped: true`, and
the key is not `caught_up` until it has.

### Switching workspaces

A change belongs to the key it was made under. `comemory auth logout` — and a
login to a different workspace — stamps every change still owed with the key
being left; under any other key those changes are held `workspace` (status:
`held.workspace`) and go out once you log back in to their own. Editing a
memory that was pulled from another workspace stamps the edit with that
workspace too, so it is never sent to the one you are logged in to. A new key
starts its own cursor and never inherits another key's `caught_up`.

### Old and new servers on one workspace

Machines on the old protocol and machines on `replica-v1` share one
workspace history: the engine journals every old-protocol import into its
replica feed and writes every replica acceptance to the old log, and a pulled
change is recorded as pulled, so it is never pushed back. The platform
forwarding of the replica routes, with repository policy applied to them, is
tracked in CodaSignal/comemory.io#183; until it ships, a platform upstream
negotiates the old protocol with `coverage_reason: "replica_unsupported"`.

An upstream that is a bare `comemory serve`, with no policy authority in
front of it, answers the policy route `404`. The exchange then runs under the
policy last recorded for that key; with none recorded it approves nothing, so
every change is held `policy` rather than sent.
