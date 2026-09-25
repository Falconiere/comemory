# Drain durable push/pull backlogs over `replica-v1` — Design

**Date:** 2026-09-24   **Status:** Approved   **Author:** Auto
**Topic:** The client half of `replica-v1`: negotiation, the push/pull drain,
cursor meaning, failure states, backoff, verification and workspace keying
(issue [#255](https://github.com/Falconiere/comemory/issues/255)).

Parent epic: Falconiere/comemory#248. Builds on
[the replica-v1 journal](2026-09-21-replica-v1-journal.md) (#250),
[memory mutation capture](2026-09-22-memory-mutation-capture.md) (#251),
[code generation replication](2026-09-22-code-generation-replication.md) (#252)
and [document revision replication](2026-09-23-document-revision-replication.md)
(#253). Those issues built the journal, the three entity kinds and the engine
routes. None of them built the client: `comemory sync` still speaks only the
legacy memory/code wire, and `domains/sync/pull.rs` has two defects this design
removes.

## Problem

1. **A pull stops short.** `run_pull` caps a run at 2,000 received entries
   (`MAX_BATCH * 4`). A watcher that got one nudge for a 3,000-entry burst
   applies 2,000 and waits for a nudge that never comes.
2. **A pull skips what it failed to apply.** `run_pull` discards the per-entry
   statuses `exchange::import::run` returns (`pull.rs:64`) and advances
   `pulled_seq` past every received entry, applied or not. The cursor records
   receipt, not application.
3. **Nothing sends the new journal.** Memory mutations have queued in
   `replica_operation` since #251 and document revisions have journalled since
   #253, but no client drains either, and a code generation is never captured
   for upload at all. Two machines on `replica-v1` would not converge.
4. **Failures are one undifferentiated error.** A revoked key, a rate limit, an
   unapproved repository, a secret, an unreadable newer kind and a dropped
   connection all surface as a failed leg that the next five-second cycle
   repeats identically, and `comemory sync --action status` shows none of them.
5. **The engine blocks the exchange it serves.** Three engine behaviors found
   while designing the client would stop two machines converging even with a
   correct client, so this issue fixes them (§ Engine changes).

## Non-Goals

1. **No platform work.** Forwarding and gating the replica routes by workspace
   and policy is CodaSignal/comemory.io#183; channel relay is #184; console
   status is #185. § Platform contract states what the client expects; every
   test here runs against the real engine.
2. **No new entity kinds.** Feedback and activity are #254. The drain is
   generic over kinds; the one thing it adds for them is reading their feed
   positions into the outbox (§ Push), because #254 journals events and
   queues nothing.
3. **No re-publication of data an upstream lost, and no pre-journal adoption.**
   After a stream replacement the client re-pulls and reports the residual
   difference; re-offering already-acknowledged operations to a restored
   upstream, minting a new epoch on every restore, and adopting memories written
   before the journal existed are #256.
4. **No daemon lifecycle change.** Residency, install and self-repair are
   #257/#258. This issue makes a daemon cycle, a watch nudge and a manual run
   drain to completion and survive failures without exiting.
5. **No pull-side vector echo.** A pulled memory is re-embedded locally from
   its text, as #251 decided; the push attaches the local vector when it
   belongs to the pushed revision so the upstream keeps its semantic search.
6. **No new legacy wire shapes.** The legacy session keeps its request and
   response shapes; it gains the unmanaged-origin rule, the drain loop in place
   of the 2,000 cap, and a pull that stops before an entry it did not apply.

## Architecture

### Where the pieces live

| Module | Owns |
| --- | --- |
| `domains/sync/drain.rs` + `drain/` | The client: session open, negotiation, the drain loop, push, code capture, pull, holds, backoff, verification, workspace binding, status |
| `domains/sync/replica/pulled.rs` | Engine half: apply ONE upstream entry to this database in upstream order |
| `store/schema_exchange.rs` + store files | Per-key session state and cursor, policy snapshots, pull holds, entity bindings |
| `store/replica_outbox.rs` | Gains hold reasons, per-key stamps, the eligible-row query and the settle/hold writers |

The client reaches the upstream only over HTTP (`drain/transport.rs`) and
applies what it pulls through `replica::pulled`. Every new file stays under the
300-code-line ceiling; `drain/` is a folder with its own README.

### The session key

Every piece of client state is keyed by **`(api_url, workspace_id)`**, where
`api_url` is `auth.json`'s base URL with the trailing `/` removed. The stream
epoch is the third part of the key for anything positional: a cursor, a pull
hold or a binding's synced sequence recorded under one epoch is void under
another. `replica_cursor` — written by nothing until now — is rebuilt with the
primary key `(api_url, workspace_id)` and also records the operation id found
at the cursor position (the **anchor**).

### Session open

```
auth.json ─▶ auth fingerprint ─▶ suspended for this fingerprint? ── yes ─▶ no network
                                   │ no
                                   ▼
                           backoff until T > now? ── yes ─▶ no network (auto/daemon/watch)
                                   │ no                          (manual honors only Retry-After)
                                   ▼
                GET /v1/sync/status ─▶ 200 valid: managed, persist snapshot
                                   ─▶ 404: unmanaged, use the persisted snapshot for this key
                                   ─▶ 401/403: suspend on this fingerprint
                                   ─▶ 200 invalid: protocol_error, no network
                                   ─▶ timeout/5xx/429: backoff
                                   ▼
          GET /v1/sync/replica/manifest (every pass) ─▶ decide (table below), persist
                                   ▼
            replica-v1 ─▶ drain (manifest gives captured head, epoch, kind capabilities)
            legacy     ─▶ legacy pull + push under the same drain loop
```

**Policy source.** A managed origin (the platform) always serves
`/v1/sync/status`; its answer is validated exactly as today, persisted as the
key's **policy snapshot**, and written to `repository_approval` as before. An
origin that answers that route `404` is an **unmanaged engine** — a bare
`comemory serve` with no policy authority in front of it. The session then uses
the snapshot persisted for this exact key; with none, nothing is approved, so
every local operation is held `policy` and every pulled entry is held. No
command writes a snapshot; tests write one through the store API a policy load
uses, the precedent #253 set for `repository_approval`. Managed responses must
echo the protocol and policy-revision headers (fail-closed, as today); an
unmanaged origin has no revision to echo, so only the body's own `protocol`
field is checked.

**Negotiation** is a pure function of three classified outcomes, persisted
once decided:

| Stored | Manifest outcome | Selected | Coverage |
| --- | --- | --- | --- |
| none/legacy | `200`, `protocol = replica-v1`, capabilities ∋ `replica-v1` | `replica-v1` (upgrade, once) | `full` |
| none/legacy | `200`, capabilities empty (upstream still seeding) | `legacy` this run | `partial` (`upstream_not_ready`) |
| none/legacy | `404` | `legacy` | `partial` (`replica_unsupported`) |
| any | `401`/`403` | unchanged | — (`auth_suspended`) |
| any | `200` with another `protocol`, capabilities that name kinds but not `replica-v1`, or an undecodable body; `409 sync_upgrade_required` | unchanged | — (`protocol_error`, no network) |
| any | timeout / `5xx` / `429` | unchanged | — (backoff) |
| `replica-v1` | anything but a valid replica manifest | `replica-v1` (never downgraded) | — (the failure's own state) |

A legacy session against an unmanaged origin uses the engine's own legacy
routes without the managed headers and without the `repositories` map (the
engine's `ImportRequest` denies unknown fields; the platform strips it). That
is what lets a real engine that has not finished seeding stand in for an old
server in tests.

**Capabilities name kinds.** The engine manifest's `capabilities` grows from
`["replica-v1"]` to `["replica-v1", "memory@1", "code_generation@1",
"document_revision@1", "feedback_event@1", "activity_event@1"]` — one
`<kind>@<schema_version>` per registered handler,
still empty until seeding completes. A client pushes only kinds the upstream
advertises; an upstream that advertises only `replica-v1` (the #250 engine) is
read as `memory@1`.

### Upgrading from the old protocol, once

The first pass after a key selects `replica-v1` (the **upgrade pass**):

- Every local-origin **document** feed row with no outbox row is enqueued
  (#253 journalled revisions but queued nothing).
- The legacy cursors are left as they are and never used again for this key.
- Memory operations the old protocol already delivered are recognized by
  content, not by cursor (the legacy push advances `pushed_seq` past rows it
  withheld or that were answered `stale`, so `pushed_seq` proves nothing): the
  pass pulls before it pushes, and the **already upstream** rule below settles
  any pending operation whose exact bytes the upstream already holds. An
  operation the upstream does not hold — withheld by the legacy push, refused,
  or never sent — is sent (or held) like any other. At selection — when the
  legacy push ever delivered anything for the workspace (`pushed_seq > 0`) —
  the key records `upgrade_through` = the captured head, and every memory
  operation created before the selection is held `upgrade` until the cursor
  reaches `upgrade_through`, however many passes that takes, so no delivered
  operation is resent before its entry has been seen. A workspace the legacy
  push never delivered to has nothing to recognize, so nothing is held. Operations created after the
  selection flow normally; a stall before `upgrade_through` keeps the older
  ones held (visible) until the stall clears.

### The drain

One pass, under `sync.lock`, alternates bounded batches, pull first:

```
open session ─▶ adopt events ─▶ capture code generations ─▶ reconsider holds ─▶ verify anchor
     ┌──────────────────────────────────────────────────────┐
     ▼                                                      │
  pull one page (limit 500) ─▶ push ≤500 ops / ≤4 MiB  ─────┘
```

A pass ends, and reports why:

| End | When | `more` |
| --- | --- | --- |
| `caught_up` | cursor ≥ captured head and no eligible operation | `false`, or `true` if a page reported a higher head |
| `budget` | `[sync] pass_budget` (default `30s`) spent before starting another batch; manual runs have no budget | `true` |
| `stalled` | the pull stalled (below) and no eligible operation is left to send | `false` |
| `no_progress` | a full pull+push iteration neither moved the cursor nor settled, held or sent anything | `false` |
| `network` | a request failed after its in-pass retries | `false` (backoff state set) |

- The **captured head** is the upstream `head_sequence` at session open. A page
  reporting a higher head means the head advanced; the pass still stops at the
  captured head and reports `more: true`.
- `more: true` makes the caller run the next pass immediately: a daemon cycle
  skips its sleep, `comemory watch` pulls again without a nudge, and a manual
  `comemory sync` keeps going. No backlog needs a second notification.
- Within one pass an operation is sent at most twice without an answer; after
  that it waits for the next pass (`attempts`, `last_error` recorded).
- **Fairness.** Pull and push alternate per batch, so a 10,000-entry history
  delays a new local write by at most one page. No SQLite write transaction is
  held across a network call: each pulled entry commits alone, each push answer
  commits per batch, and derived graph state is refreshed once per page rather
  than per entry. The inline push after a save takes `sync.lock` with `try`
  and skips when a pass holds it; the running pass sends the new operation on
  its next push batch.
- A pass killed at any instant loses nothing: every state change is one
  committed transaction, and the next pass resumes from what committed.

### Push

**Eligible rows:** `replica_operation` with `state = 'pending'` and
`hold_reason IS NULL`, ordered by `(created_at, rowid)` — the order this device
made them — each under its original `operation_id` and the bytes
`replica_payload` holds for its digest, so a retry is byte-identical. Holds are
re-evaluated at the start of every pass, so a held row never occupies the head
of the queue.

At first send, two wire fields are resolved and **persisted** on the row, so
every retry carries the same values:

- `wire_repository` — the canonical `owner/name` the snapshot resolves the
  row's local label to (the platform gates on it; the payload keeps its label).
- `observed_sequence` — for a `restore`, the upstream sequence of the tombstone
  this key last applied or had accepted for the entity. A restore whose
  tombstone is in the same batch ends that batch, and goes in the next one once
  the tombstone's sequence is known. A restore with no known tombstone sequence
  (nothing bound yet, as right after an upgrade) is held `order` until a pull
  binds one; it is never sent with a null.

**Events.** `feedback_event` and `activity_event` positions are journalled
and never queued (#254): the feed position is their durable record. Before
every push the drain runs #254's capture — until it is caught up on a full
pass, one batch on the inline push (nothing reads a client's feed, which is
where the engine runs it) — then reads local event
positions of each kind above that kind's `schema_meta` cursor
(`replica_event_adopted_through:<kind>`) and enqueues each one whose operation
the outbox does not already hold, one page per transaction with the cursor
that covers it. The inline push after a save adopts at most one page per kind;
the cursors carry the rest to the next pass. Logout and a switching login run
the same capture and adoption before they stamp (§ Keying). Adoption is idempotent, and the
capture batch skips a run that already carries an event id, so a cursor a
rebuild resets costs a rescan, never a second upload. From there an event is
an outbox row like any other.
On the pull side an event this engine already holds is answered `duplicate`
whatever operation id carries it, and never counted twice.

Before a row is sent it is classified; a hold is written to the row
(`hold_reason`), never to the upstream:

| Hold | When | Reconsidered |
| --- | --- | --- |
| `policy` | no approved canonical name for the label in the snapshot; or the upstream answered `rejected_not_allowed` | each pass, when the label now resolves |
| `secret` | memory body matches a secret rule without an override | when `--allow-secret` records one |
| `skip_repos` | label matches `[sync] skip_repos` | each pass |
| `workspace` | the row or its entity is stamped with another key (§ Keying) | when the session key is the stamped key |
| `incompatible` | the upstream does not advertise the kind/version | when the manifest advertises it |
| `order` | an earlier row for the same entity is held | when that row is no longer held |
| `upgrade` | memory row created before this key selected `replica-v1` | when the cursor reaches `upgrade_through` |

`order` keeps one entity's changes in the order made; other entities are
unaffected, so a blocked repository never starves an approved one.

Batches hold at most 500 operations and at most `[sync] max_request_bytes`
serialized (default 4 MiB, under the engine's 5 MiB envelope and body limits;
an operator behind a proxy with a smaller body limit lowers it). A single
operation over the limit is sent through `stage` (parts of at most a quarter of
the limit) and `activate`, whatever its kind. A `413`
or `400` for a batch splits it in halves repeatedly; a single operation still
refused is marked `rejected` with the upstream's message.

Answers, per operation:

| Disposition | Row becomes |
| --- | --- |
| `accepted`, `duplicate` | `accepted`, with upstream sequence and epoch; entity bound to the key with that digest and sequence |
| `rejected_not_allowed` | still `pending`, held `policy` (the platform refuses before the engine writes a receipt, so the same id can succeed later) |
| `rejected_stale`, `rejected_conflict`, `rejected_invalid`, `rejected_unsupported`, `payload_erased`, `payload_expired` | `rejected`, disposition and reason kept, visible |
| no result for the operation, timeout, transport error, `5xx` | still `pending`, `attempts + 1`, `last_error` |

A held or pending operation is never shown as synchronized.

**Code generations are captured at push time**, as #252 designed. For every
repository the old code push would offer (approved canonical identity, not a
worktree, root present, not skipped, `[sync] code_index` on) with no pending
`code_generation` operation for it: plan; if the plan differs from the active
generation, record it `staged` (origin local), journal it and enqueue it in one
transaction, keyed by the canonical repository. On `accepted` it is activated
locally; on `rejected_stale` it is left staged and the next pass replans on top
of whatever the pull activated. `gc` no longer sweeps a staged generation that
still has a pending operation.

**Documents enqueue at capture.** `domains::documents::journal` enqueues the
outbox row in the transaction that journals the revision.

### Pull

`GET /v1/sync/replica/changes?since=<cursor>&limit=500&epoch=<epoch>`. The
cursor after a page is **always the page's `next_sequence`** when it has one —
never the last entry's sequence — so a page emptied by a server-side filter
still advances by the server's raw continuation. On an unfiltered pull from a
managed origin, positions skipped between consecutive entries are recorded as a
`server_withheld` hold under the policy revision the request carried.

Each entry, in order; the cursor moves only past positions resolved by one of
these rows:

| # | Case | Action | Cursor |
| --- | --- | --- | --- |
| 1 | `operation_id` is one of this client's outbox operations | settle it `accepted` (an acknowledgement arriving by pull), bind, do not apply | advances |
| 2 | kind or schema version this build cannot read | **stop**: `stalled_at`, `incompatible_version` | stays before it |
| 3 | sequence ≤ the entity's bound `synced_sequence` for this key and epoch | superseded, skip | advances |
| 4 | repository not approved by the snapshot (re-read before every page) | hold `policy` with the repository | advances |
| 5 | memory whose pending operations include one with this entry's exact digest (or a tombstone meeting a pending tombstone) | **already upstream**: settle the OLDEST such operation and every older pending one for the entity `accepted` (`already_upstream`), bind; newer pending operations stay | advances |
| 6 | memory with any other pending local operation | hold `pending_local` | advances |
| 7 | upsert or restore whose payload is `erased` or `absent` | skip, reason recorded | advances |
| 8 | otherwise `replica::pulled::apply` → applied or duplicate | bind entity with digest and sequence | advances |
| 9 | `pulled::apply` refuses as invalid, or fails (I/O, SQLite) | **stop**: `stalled_at`, the reason | stays before it |

`replica::pulled::apply` is the engine's acceptance path in upstream order: a
receipt replay answers `duplicate`; kind, shape and identity are validated; the
erasure barrier stands; then `materialize` writes state, feed row (`origin =
sync`, never enqueued, under the entry's own `operation_id`) and receipt in one
transaction. It skips `order()`, whose observed-sequence check is in the
upstream's sequence space, and for a code generation it skips `code_accept`'s
parent check — the upstream already ordered the entry, so a pulled generation
activates whatever this machine had active, and a pending local generation for
the repository is answered stale upstream and replanned. It records no receipt
for a refusal, so a stall succeeds once its cause is fixed.

**Reconsidering holds.** At the start of a pass a hold is due when its cause is
gone: a `policy` hold whose repository now resolves, a `server_withheld` hold
whose policy revision differs, a `pending_local` hold whose entity has no
pending operation left. The cursor rewinds to one before the earliest due hold
and the due holds are dropped; the entry at the new position is fetched and
recorded as the anchor. Replay is safe because rule 3 runs before any apply: an entry older than what this key last applied or had accepted for its
entity is skipped, so a rewind can never put an older revision over a newer
one, and an applied entry answers `duplicate` from its receipt.

**The anchor.** The cursor records the `(sequence, operation_id)` of the entry
at its position when it has one. Each pass first re-reads
`since = cursor - 1, limit = 1`. The same sequence with a different
`operation_id` means the stream was rewritten under the same epoch, and the pass
rebootstraps (below). A different sequence (the position was filtered, or a
filter now reveals it) means the anchor is unknown: it is recorded afresh, and
nothing is rebootstrapped.

**Legacy pull.** On a legacy key the cursor never passes an entry the import
failed to handle: `accepted`, `exists`, `stale` and `deleted` advance;
`secret_detected`, `repo_not_allowed` and `id_collision` are refusals with no
local remedy (the import scans pulled bodies with no override), so they advance
as visible holds; `invalid` and an I/O or SQLite failure stall, and
`pulled_seq` names the last entry before it. The remedy for an `invalid` stall
is an upstream fix; status names the entry. Legacy holds live in
`replica_pull_hold` under `stream_epoch = 'legacy'` with reasons `policy`,
`secret` and `id_collision`. They are counted, never rewound: the legacy import
has no pending-edit or ordering guard beyond tombstones, so a rewind would
re-patch memories edited since. A legacy key keeps today's remedy after a policy
change, the existing `verify` repair, now run through the drain loop instead
of its 2,000-entry cap. That repair resets both legacy cursors and re-imports
everything, so it can re-patch a memory edited locally since: a known legacy
hazard this issue documents and does not change. Hold reconsideration is a
`replica-v1` guarantee.

### Failure handling and backoff

Every request carries a timeout (`[sync] request_timeout`, default `30s`; the
inline push keeps `push_on_save_timeout`). Within a pass a failed request is
retried at most twice, with full-jitter delays between 1 s and 8 s. Across
passes the key's state persists:

| Upstream answer | State | Next network attempt |
| --- | --- | --- |
| `429` + `Retry-After` (seconds or HTTP date, capped at 1 hour) | `backoff` | not before `Retry-After`, manual runs included |
| timeout, connection refused, `5xx` | `backoff` | full jitter up to `min(2^n s, 5 min)` after `n` consecutive failures; manual runs retry at once |
| `401`, `403` | `auth_suspended` | only once `auth.json`'s fingerprint changes |
| `409 sync_policy_changed` | reload the policy, reclassify, retry once | now, then `backoff` |
| `409 epoch_mismatch`, `409 cursor_ahead`, anchor mismatch | rebootstrap (below) | now |

`auth_suspended` stops network work only. The daemon keeps looping, local
writes keep journalling, and status keeps reporting what is pending.

### Verification and rebootstrap

`replica_binding` records, per key, each entity this key has exchanged: the
digest the upstream last held for it (from a pulled entry or our accepted
operation), whether it is deleted, and the upstream sequence and epoch of that
revision. `comemory sync --action verify` on a replica key compares, per kind
the upstream manifest lists, the upstream's 256 bucket digests against the
same buckets over the key's bound live digests. The local side is the upstream
as this client last knew it, so a pending local edit does not read as a
mismatch.

A differing kind is repaired by a **compacting replay** of
`changes?kind=<kind>&since=0` without moving the main cursor, then compared
again. Differences that remain are reported beside the key's held counts, not
looped on.

**Compacting replay.** Every replay from sequence 0 — a rebootstrap and a
verify repair — runs in two phases so an older entry can never land over a
newer one, whatever the bindings say:

1. *Scan* pages from 0 to the replay target (the captured head) and record,
   per entity, only its last position (sequence, operation id, digest, payload,
   repository) in the scratch table `replica_replay`, classifying holds as the
   forward rules do. Rules 1 and 5 also run at EVERY scanned position, because
   they only settle outbox rows: an operation delivered at an earlier position
   is settled even when a later entry is the entity's last. No domain state is
   written.
2. *Apply* each entity's last position in sequence order through rules 1–9,
   except that when the entity has no pending local operation and its local
   revision digest differs from the entry's, rule 1 (own operation) and a
   `duplicate` answer **rewrite** local state to the entry's payload, journalled
   under a fresh operation id with `origin = sync`. Bindings take the entry's
   sequence and digest. Each applied entity's scratch row is deleted in the
   transaction that applies it.

A replay is resumable, and a key runs one at a time. `sync_exchange` records
`replay_kind` (`rebootstrap`, or `repair` with its entity kind),
`replay_state` (`scanning` or `applying`), `replay_scan_through` and
`replay_target`. When the apply phase empties the scratch table, a
`rebootstrap` sets the main cursor to the target; a `repair` never moves the
main cursor, since other kinds' positions between it and the target are still
owed to the forward pull. A pass that
finds a replay in progress — after a kill, a budget end or a network failure —
resumes it in the same phase instead of pulling forward, and `upgrade` holds
are cleared only when no replay is in progress and the cursor has reached
`upgrade_through`.

A rewind for due holds is not a replay from 0: bindings keep their
`synced_sequence`, so rule 3 alone prevents regression there.

An upstream whose epoch differs from the cursor's (`409 epoch_mismatch`, or a
manifest naming another epoch), whose head is below the cursor (`409
cursor_ahead`), or whose anchor moved, is a replaced stream: the cursor
restarts at `(epoch, 0)`, every pull hold for the key is dropped and every
binding's `synced_sequence` for the key is cleared (whatever the epoch, since a
same-epoch restore reuses sequences) while its digests stay for verify; the
restored stream is then read by a compacting replay, pending operations stay
pending, and the report says `rebootstrapped: true`. Nothing local is deleted. An upstream restored under
the same epoch that has already written past the cursor, over positions whose
anchor was never recorded (an empty filtered page), cannot be detected by a
client; #256 owns minting a new epoch on restore.

### Keying, workspace switches and mixed clients

- **Operations belong to the key they were made under.** A row is stamped with
  a key at its first send. `auth logout` stamps every unstamped pending row
  with the outgoing key before the credential is removed; a login that changes
  the key does the same before the new credential is written. A session whose
  last-used key differs from its own (a credential replaced by hand) stamps the
  unstamped rows created before the last session under that key ended. A
  stamped row is sent only to its key; under any other it is held `workspace`.
  A feedback or activity event enters the outbox when a pass adopts it, so
  logout and a switching login capture and adopt events before they stamp.
  With a credential replaced by hand, an event not yet adopted when the
  credential changed belongs to the key whose pass adopts it, like any row
  made after the old key's last session ended.
- **Entities belong to the key they came from.** `replica_binding` is keyed by
  `(api_url, workspace_id, entity_kind, entity_key)`. A local edit of an entity
  bound only to other keys is stamped with the most recently bound one and held
  under the current key, so editing a memory pulled from one organization while
  logged into another never sends it there. An entity bound to the current key
  (or to none) is stamped with the current key.
- **One protocol per key.** Selection is persisted and only ever moves from
  `legacy` to `replica-v1`. An old-protocol client and a `replica-v1` client
  on one workspace converge on the engine's single history: the legacy import
  also journals the replica feed, the replica accept also writes `sync_log`,
  and a pulled change is journalled `origin = sync`, so it is never pushed back.

### Engine changes

| Change | Why |
| --- | --- |
| `materialize` journals an accepted memory under the incoming `operation_id` (it minted a fresh one; code and documents already keep theirs) | A client must recognize its own operation in the pulled feed, and the upstream feed must carry the ids the device made |
| `materialize` soft-deletes the mirror row for a tombstone unconditionally | A replay after a kill between the markdown move and the commit found no file, skipped the soft-delete and left a live row |
| `materialize` leaves the derived-graph refresh to its caller; the import route refreshes once per envelope, the pull once per page | Refreshing after every entry makes a 3,000-entry pull quadratic |
| Journal seeding journals without enqueueing | Seeding queued every pre-journal memory as an upload owed by the upstream, and an owed upload makes the engine refuse every import for that entity |
| The pending-upload refusal in `validate::decide` applies only on an engine that has a `replica-v1` upstream of its own — any `sync_exchange` row whose `protocol` is `replica-v1` | An engine nobody is a client of (the hub) owes no uploads; its console and HTTP writes would otherwise freeze every client edit of the same entity |
| New operation ids carry 128 bits of a SHA-256 digest over the entity, the operation, the clock, the process and a per-process counter (`op-<yyyymmdd>-<32hex>`), from their own minter; `dated_id` and the `q-`/observation shapes are unchanged; existing operation ids stay valid and no engine validates the shape | Client-minted ids become the hub's feed keys across a whole workspace; 32 bits a day collide (`rejected_conflict`) at a few tens of thousands of operations |
| On the pull path a code generation skips `code_accept`'s parent check (`replica::pulled`); the import route keeps it | A client follows the order the upstream already chose; the parent check exists to order two machines' concurrent pushes at the upstream |
| `comemory rebuild` copies the new tables and columns | A rebuild must not forget stamps, holds, bindings or the anchor, or stamped rows would flow to another key and rule 3 would lose its protection |
| `changes?kind=` scans `limit` raw positions and returns the matches with `next_sequence` = the last position scanned | The platform's filtered-page semantics; a kind-scoped replay then advances through empty pages by the raw continuation |
| `capabilities` lists `<kind>@<version>` | Kind negotiation |

## Interfaces / Schema

### Migration `0027_replica_exchange`

| Table / column | Key | Holds |
| --- | --- | --- |
| `sync_exchange` | `(api_url, workspace_id)` | `protocol`, `coverage_reason`, `selected_at`, `upgrade_through`, `replay_kind`, `replay_state`, `replay_scan_through`, `replay_target`, `network_state`, `retry_at`, `consecutive_failures`, `last_error`, `suspended_fingerprint`, `upstream_head`, `stall_sequence`, `stall_reason`, `last_session_at`, `last_ok_at` |
| `sync_policy_snapshot` | `(api_url, workspace_id)` | `revision`, `fingerprint`, `allowlist_json`, `mappings_json`, `loaded_at` |
| `replica_cursor` (rebuilt; never written before) | `(api_url, workspace_id)` | `stream_epoch`, `applied_sequence`, `anchor_sequence`, `anchor_operation_id`, `updated_at` |
| `replica_replay` (scratch) | `(api_url, workspace_id, entity_kind, entity_key)` | the entity's last `sequence` and that whole pulled entry as `entry_json` |
| `replica_pull_hold` | `(api_url, workspace_id, stream_epoch, from_sequence)` | `to_sequence`, `reason`, `entity_kind`, `entity_key`, `repository`, `policy_revision`, `recorded_at` |
| `replica_binding` | `(api_url, workspace_id, entity_kind, entity_key)` | `synced_digest`, `synced_deleted`, `synced_sequence`, `synced_epoch`, `updated_at` |
| `replica_operation` + | — | `hold_reason`, `hold_detail`, `api_url`, `workspace_id`, `wire_repository`, `upstream_epoch` |

`replica_operation.state` keeps its three values; a hold is a reason on a
`pending` row, which is what keeps `has_pending_for` protecting the local edit.

### Config

| Key | Default | Role |
| --- | --- | --- |
| `[sync] request_timeout` | `"30s"` | Per-request budget for every sync call except the inline push |
| `[sync] pass_budget` | `"30s"` | Time after which a daemon, auto or watch pass starts no new batch and reports `more: true` |
| `[sync] max_request_bytes` | `4194304` | Largest serialized push request; an operation over it crosses through `stage`/`activate` |

### Wire

Requests carry `x-comemory-sync-protocol: replica-v1` and
`x-comemory-policy-revision: <snapshot revision>`. Engine manifest
`capabilities` gains `<kind>@<version>` entries (additive; clients ignore
unknown strings). `Operation.repository` carries `wire_repository`.

### Status JSON

`comemory sync --action status --json` keeps every existing field and adds:

```json
"exchange": {
  "api_url": "https://api.comemory.io",
  "workspace": "ws_…",
  "protocol": "replica-v1",
  "coverage": "full",
  "coverage_reason": null,
  "network": "ok",
  "retry_at": null,
  "consecutive_failures": 0,
  "last_error": null,
  "stream_epoch": "…32 hex…",
  "applied_sequence": 4210,
  "upstream_head": 4210,
  "caught_up": true,
  "outbox": { "pending": 0, "retryable": 0, "held": { "policy": 0, "secret": 0,
              "skip_repos": 0, "workspace": 0, "incompatible": 0, "order": 0,
              "upgrade": 0 },
              "rejected": 0 },
  "pull": { "held": { "policy": 0, "pending_local": 0, "server_withheld": 0,
                      "secret": 0, "id_collision": 0 },
            "stalled_at": null, "stall_reason": null }
}
```

`network` is `ok`, `backoff`, `auth_suspended` or `protocol_error`. `caught_up`
is true only when the cursor's key and epoch match the session, the cursor
equals the last seen upstream head, no replay is in progress, nothing eligible
is pending, no pull is stalled and the network state is `ok` — never across a key or epoch change. A
legacy key has no replica cursor, so it is never `caught_up`.
On a replica key a `run`/`push`/`pull` report replaces its `push`/`pull`/`code`
legs with one `exchange` leg: `pushed`, `pulled`, `held`, `settled`,
`rejected`, `end`, `more`, `rebootstrapped` and `network`. A manual run whose
drain ended on the network writes that report, then exits `69`
(`EX_UNAVAILABLE`) with the key's recorded error; unattended passes exit 0. `verify --json` on a
replica key reports per kind `{kind, differing_buckets, repaired}` plus
`held_positions`.

### Platform contract (for #183)

The platform forwards `/v1/sync/replica/{manifest,changes,import,stage,activate}`
with the managed headers echoed; filters `changes` by policy while keeping
`next_sequence` as the raw scanned continuation; answers an unapproved
repository's operation `rejected_not_allowed` without writing an engine
receipt; answers a stale revision `409 sync_policy_changed`; and never
forwards `/v1/sync/status`.

## Failure modes and edge cases

| Input / failure | Behavior |
| --- | --- |
| 3,000 pending remote entries, one nudge | One pass drains to the captured head; `more` covers what arrived meanwhile |
| Several pages all held by policy, then an allowed entry | Allowed entry applied; holds recorded; cursor never stops on an empty result |
| Kind-scoped replay over a feed where the kind is sparse | Empty pages advance by `next_sequence` until the head |
| Memories directory unwritable mid-page | Stall at that entry, pass ends `stalled`; fixed → replay applies it, earlier ones are skipped or answer `duplicate` |
| Upstream entry of a kind this build cannot read | Pull stalls before it, pass ends `stalled` (pushes still go out); nothing after it applies until upgrade |
| Push response dropped; client killed | Next pass pulls first, recognizes its own operations (rule 1) and settles them; unrecognized ones are resent with the same ids and bytes |
| Older acknowledgement arrives (by pull) after a newer local edit | Rule 1 settles the older operation; the newer stays pending and the local file keeps its text |
| Held entry replayed after a newer revision was applied | Rule 3 skips it |
| Repository revoked while a pull request is in flight | The page is judged under the snapshot re-read on arrival: revoked entries are held, not applied |
| Approval restored | The holds are due; the cursor rewinds; the entries apply unless superseded |
| Secret in a memory | Held `secret`, never sent, counted; other memories flow |
| Restore after a pushed delete | Restore carries the tombstone's upstream sequence; accepted |
| `429` with `Retry-After: 2` | No request for 2 s from any pass; then the drain continues |
| Engine stopped | `502`/connection refused → jittered backoff, bounded attempts; restart → converge, no duplicate effects |
| Key rotated (`401`) | `auth_suspended`; no requests until `auth.json` changes; pending still counted |
| Upstream restored from an earlier copy | `cursor_ahead` or anchor mismatch → rebootstrap; pending kept; verify reports what the upstream lost |
| Logged out of A, credential for B | A's pending rows held `workspace` under B, sent after switching back to A |
| Console edit on the hub, then a client edit of the same memory | Accepted in server order; the hub owes no upload |
| Pass killed mid-page | Every committed entry stays applied; the cursor names the last one |
| Two passes race (hook + daemon) | `sync.lock` serializes them; the inline push skips when a pass holds it |
| Legacy pull meets an `invalid` entry | Stalls before it; `pulled_seq` names the entry before |
| Legacy pull meets a `secret_detected` entry (a teammate's override) | Counted as a held entry and passed; every other entry keeps flowing |
| Hub restored under the same epoch, then written past the old cursor | Detected only if the anchor moved; #256 owns a new epoch on restore |
| Pending `D1`, `D2`, then a revert back to `D1` | Rule 5 settles the oldest `D1`; the revert stays pending and is sent |
| Restored stream: peer edit of M at X, our accepted op for M at X+1 | Compacting replay applies only X+1 and rewrites local M to it |
| Upgrade pass ends on `budget` before `upgrade_through` | Pre-upgrade memory rows stay held `upgrade` on later passes until the cursor passes it |
| Pulled code generation whose parent is not the local active one | Activated in upstream order; a pending local generation is answered stale and replanned |

## Acceptance criteria

- **AC-1:** (X-1) Against a real engine that advertises `replica-v1`, `comemory
  sync` selects `replica-v1`, persists it for `(api_url, workspace)`, and status
  reports `protocol: "replica-v1"`, `coverage: "full"`; the manifest's
  capabilities list `memory@1`, `code_generation@1`, `document_revision@1`,
  `feedback_event@1` and `activity_event@1`.
- **AC-2:** (X-1, X-8) Against a real engine still seeding (250 memories
  restored by a real `comemory rebuild`; seeding runs 200 per replica call and
  the client's run makes exactly one manifest call), the client selects
  `legacy` with `coverage: "partial"`, `coverage_reason: "upstream_not_ready"`,
  and syncs memories through that engine's legacy routes, one of them withheld
  by `skip_repos`. When the engine advertises, the next run upgrades once: the
  operations the legacy push delivered are settled `already_upstream` and the
  engine's feed gains no second position for them, while the withheld one is
  still held, not settled.
- **AC-3:** (X-1, X-6) A `401` from a restarted engine's rotated token leaves
  the selection unchanged and sets `network: "auth_suspended"`; a running
  `comemory sync daemon run` process stays alive through at least three cycles
  and sends no request (fault-proxy log) while `outbox.pending` stays visible in
  status; rewriting `auth.json` with the new token makes that same daemon
  process resume and drain without a restart. A manifest response whose protocol value the proxy
  corrupts in transit sets `protocol_error`, sends nothing further, and never
  selects `legacy` — neither for a fresh key nor for one already on
  `replica-v1`.
- **AC-4:** (X-3) More than 2,000 real operations made on A through its HTTP API
  (saves and edits built from this repository's `docs/guides/*.md`) drain to B
  after one `comemory sync` on A and ONE `comemory sync --action auto` on B (a
  single hook nudge): B holds the last operation's state, A's `outbox.pending`
  is 0, B's `applied_sequence` equals the upstream head, and no page exceeded 500
  entries. An `--action auto` invocation keeps running passes while one reports
  `more: true`.
- **AC-5:** (X-3) With `pass_budget = "1s"` and 3,000 upstream entries, an
  `--action auto` pass ends `budget` with `more: true`, and `comemory sync daemon
  run` keeps cycling without sleeping until `caught_up`, with no nudge. A
  `comemory save` on A during a long drain returns within 5 s (its inline push
  finds `sync.lock` held and skips) and its operation is accepted upstream by
  that same drain. Entries written to the hub while a pass runs are drained by
  the pass that follows it, with no nudge. A daemon sent `SIGTERM` mid-drain
  exits within 2 s, and a drain killed with `SIGKILL` mid-page resumes on
  restart with no lost and no duplicated effect (receipt, feed and file
  counts).
- **AC-6:** (X-3, X-4) Three full pages of entries from a repository the
  snapshot does not approve, followed by one approved entry: the approved entry
  is applied in the same pass, the held positions are recorded with reason
  `policy` and their repository, and after the repository is approved the next
  pass applies them without a nudge.
- **AC-7:** (X-4) A pull whose memory write fails midway (the memories directory
  made read-only) stops before the failed entry and ends `stalled`: earlier
  entries are applied, status shows `stalled_at` and the reason, the cursor names
  the last applied entry. After write access returns, the replay applies the
  rest and the accepted prefix gains no second feed row, receipt or file. On
  the push side, a batch the hub answered partly `accepted` and partly
  `rejected_stale`, whose response was dropped, is resent whole and every
  operation gets its original answer back (accepted ones as `duplicate` with
  their original sequences).
- **AC-8:** (X-4) An upstream entry of a kind/version this build cannot read
  (written with the real journal API, as a newer engine would) stalls the pull
  before it with `stall_reason: "incompatible_version"` and the pass ends
  `stalled` with `more: false`; entries before it are applied, nothing after it
  is, and local operations are still pushed.
- **AC-9:** (X-2) A patches one memory's frontmatter twice through its HTTP API.
  The response carrying the first operation's acknowledgement is dropped and the
  client is killed. After restart the upstream feed holds both operations once
  each, in the order made, under their original ids. When the first operation's
  acknowledgement reaches A by pull after a third local patch, only the first is
  settled and the local file keeps the third patch until its own operation is
  accepted. A peer's patch to the same memory, pulled while A's patch is
  pending, does not overwrite A's file; once A's operation is accepted, both
  machines hold A's patch, in server order.
- **AC-10:** (X-5) One local store holds operations for an approved repository,
  an unapproved one, a secret-bearing memory, a `skip_repos` label and a kind the
  upstream does not advertise (an upstream whose manifest omits it). The approved
  operations are accepted; each other one stays `pending` with its own `held.*`
  count; none is ever marked accepted; a later approval sends only the newly
  eligible ones. A row sent while the hub is stopped is counted `retryable` and
  stays pending. Every held and retryable row, and its count, survives a restart
  of the client process and a `comemory rebuild`.
- **AC-11:** (X-6) A `429` with `Retry-After: 2` from the fault proxy: no request
  reaches it for 2 s, then the drain completes. With the hub stopped the fault
  proxy answers `502 Bad Gateway`: a pass makes at most three attempts per
  request, records `network: "backoff"`, and two consecutive failing passes set
  `retry_at` inside the documented full-jitter windows (`≤ 2 s`, then `≤ 4 s`
  after the failure); an `--action auto` pass started before `retry_at` sends
  nothing (proxy log). After the hub restarts the drain converges with no
  duplicate effect.
  A response held past `request_timeout = "2s"` is retried under the same
  operation ids.
- **AC-12:** (X-5, X-6) The approval for a repository is revoked while a pull
  request is held in flight: none of that repository's entries are applied, and
  no push request made while it is revoked carries its operations (proxy log).
  Restoring approval applies the held entries and sends the held operations.
- **AC-13:** (X-7) After a drain of memories, a code generation and document
  revisions, `comemory sync --action verify --json` reports zero differing
  buckets for every kind. After the upstream is restored from a copy taken
  earlier (same epoch, lower head) and, separately, replaced by a new stream
  (new epoch), the next pass reports `rebootstrapped: true`, keeps every pending
  operation and pushes it, status is never `caught_up` in between, and verify
  reports the difference the upstream lost rather than a match. A patch another
  client makes to a bound memory after the restore (landing at a reused
  sequence) reaches this client, and a memory whose restored stream holds a
  peer edit followed by this client's accepted operation ends with this
  client's text locally, matching its binding. A client killed with `SIGKILL`
  during the replay's scan and again during its apply phase resumes the replay
  in compacting mode and ends in the same state. The document revisions pushed after the copy
  was taken are the lost difference; the verify repair replays
  `document_revision` through the engine's empty filtered pages (1,500 memories,
  3 documents) to the head.
- **AC-14:** (X-8) Rows made while bound to workspace A are never sent to B
  after a real `comemory auth logout` and a credential for B written as a login
  would (proxy log and B's feed); status counts them `held.workspace`; restoring
  A's credential sends them to A. A memory pulled from A and patched under B is
  held, not sent to B. B's cursor and selection start fresh, with no `caught_up`
  inherited from A. (Proves the logout and replaced-credential stamping rules;
  the login rule shares the logout code path.)
- **AC-15:** (X-8) A legacy import to the engine (the body the platform
  forwards) and a replica push from a client converge: the replica client pulls
  the legacy write once and never pushes it back, and its own push appears in
  the engine's legacy `sync/changes` exactly once.
- **AC-16:** (X-2, X-3) A indexes a pinned checkout of this repository's
  sources and a copy of `docs/guides`; after drains on A and B, B's `comemory
  repos` shows the repository as shared at A's generation and `comemory search
  --only document` returns a shared passage. With `[sync] max_request_bytes`
  lowered to 256 KiB, the real generation of a pinned checkout of this
  repository's whole `src/` (about 0.9 MiB, asserted larger than the budget
  first) crosses through `stage`/`activate`.
  A memory deleted and restored on A is live on B, and the upstream accepted the
  restore.
- **AC-17:** (engine) A memory saved and patched through the hub engine's own
  HTTP API, then patched by a client, is accepted in server order; seeding 250
  pre-journal memories leaves the hub's outbox empty; a tombstone replayed after
  its markdown move leaves no live mirror row; after `comemory rebuild` a
  client's stamps, holds, bindings, cursor and anchor are intact and a stamped
  row still goes only to its key.
- **AC-18:** (docs, CI) `scripts/replication/coverage.json` maps `X-1`…`X-8` to
  case `exchange`; `bash scripts/test-replication-e2e.sh --case exchange` runs
  the new suites; the status JSON, retry/repair semantics, repository-policy
  compatibility and cloud-sync guide are updated.
- **AC-19:** (X-2, X-3) A verdict recorded on A (a real `find` then
  `feedback --used`), and A's scoped runs, reach B through the hub on one
  `comemory sync` each: B holds the verdict under A's event id and counts it
  once, and every run A shared arrives once. Two more passes both ways — A
  pulling its own events back among them — change no count on either side,
  and the hub holds each event once. An event pulled again under another
  operation id is answered `duplicate` and counted nothing.

## Acceptance evidence

All live checks drive real spawned `comemory serve` engines (the hub, and each
client's own `serve` for HTTP writes), the real CLI, real SQLite and real HTTP
through `tests/common/fault_proxy.rs`, a TCP proxy that forwards bytes
unchanged unless a case asks it to drop one response, hold one request until
released, delay one response, flip one byte of one response, refuse above a rate with `429`, or
answer `502` while its upstream is down. Every success body comes from the
engine. Tests use `[sync] pass_budget` and `request_timeout`, never a mocked
clock.

| AC | Real input | Expected result | Boundary | Check |
| --- | --- | --- | --- | --- |
| AC-1 | Fresh hub; memories saved by the CLI | `replica-v1`/`full`; five kind capabilities | Selection survives a second run | `replica_exchange` `negotiation_selects_replica` |
| AC-2 | 250 memory markdown files replayed by `comemory rebuild` into the hub's data dir; one client memory under a `skip_repos` label | `legacy`/`partial`, one upgrade, no second feed position, skipped one still held | Upgrade never reverses | `replica_exchange` `old_server_partial_then_upgrade_once` |
| AC-3 | Hub restarted with a new token; proxy flips a byte of `replica-v1` in one manifest response | `auth_suspended`, zero proxied requests; `protocol_error`, no legacy | Status still counts pending | `replica_exchange` `auth_and_protocol_failures_never_downgrade` + table test in `drain/tests/negotiate.rs` |
| AC-4 | >2,000 HTTP writes built from `docs/guides/*.md` | pending 0, last state on B, cursor = head | Page ≤ 500 | `replica_exchange_2` `large_backlog_drains_on_one_run` |
| AC-5 | 3,000 upstream entries, `pass_budget = "1s"`; a `comemory save` mid-drain | `budget`/`more`, daemon catches up with no nudge; save < 5 s; `SIGKILL` resume | Receipt/feed/file counts unchanged by resume | `replica_exchange_2` `budgeted_pass_reschedules_and_yields` |
| AC-6 | Memories labelled with an unapproved and an approved repo | allowed entry applied; holds recorded; approval applies them | Cursor never stops on an all-held page | `replica_exchange_3` `held_pages_then_allowed_entry` |
| AC-7 | Read-only `memories/` mid-pull | stall, pass ends `stalled`; replay applies rest once | No duplicate receipt/feed/file | `replica_exchange_3` `failed_import_stalls_then_replays_safely` + `a_resent_mixed_batch_gets_its_original_answers` |
| AC-8 | Hub feed row of a `future_kind@1` this build does not know, written with `replica_journal::append` | stall before it, `incompatible_version`, `more: false` | Pushes still sent | `replica_exchange_3` `unknown_kind_stalls_the_cursor` |
| AC-9 | Three frontmatter patches of one memory via A's HTTP API; proxy drops one import response; `SIGKILL` | original ids once each, in order; late ack settles only its op; newest patch kept | — | `replica_exchange_4` `dropped_ack_and_kill_keep_ids_and_order` |
| AC-10 | Mixed store: approved, unapproved, secret, skip, unadvertised kind | per-hold counts; approved accepted; none held marked accepted | Later approval sends only newly eligible ones | `replica_exchange_4` `held_states_do_not_starve_eligible_repos` |
| AC-11 | Proxy `429 Retry-After: 2`; proxy `502` while the hub is stopped, then restarted; response held > 2 s | no request inside `Retry-After`; ≤ 3 attempts, `retry_at` inside each jitter window, no request before it; same ids on retry | Consecutive-failure growth | `replica_exchange_5` `rate_limit_gateway_and_timeout` |
| AC-12 | Approval revoked while proxy holds a pull request; restored later | nothing applied/sent for revoked repo; restore applies/sends | Proxy log inspected for op ids | `replica_exchange_5` `revocation_during_request_holds` |
| AC-13 | Full drain of three kinds; hub restored from an earlier copy (1,500 memories, docs pushed after the copy); a peer patch after the restore; hub replaced | verify 0 differing; `rebootstrapped`; pending kept; never falsely caught up; post-restore patch arrives; kind replay crosses empty pages | Residual difference reported | `replica_exchange_6` `verify_and_rebootstrap_after_stream_changes` + `rebootstrap_keeps_the_clients_own_accepted_edit_over_a_peer_edit` |
| AC-14 | Real logout of A, credential for B, back to A | B receives none of A's rows; `held.workspace`; A receives them | B never inherits `caught_up` | `replica_exchange_6` `workspace_switch_never_crosses` |
| AC-15 | Legacy import body posted to the hub; replica client on same workspace | pulled once, not pushed back; appears once in legacy changes | — | `replica_exchange_6` `legacy_and_replica_clients_converge` |
| AC-16 | Pinned checkout of this repo's whole `src/` with `max_request_bytes = 262144` (payload asserted larger first); copy of `docs/guides`; delete + restore | shared repo in `repos`; shared passage; stage/activate used; restore accepted | Local rows untouched | `replica_exchange_7` `code_documents_and_restore_drain_both_ways` |
| AC-17 | Hub HTTP save + patch, then client patch; 250 rebuilt memories; tombstone replay state produced with the real library API; client `comemory rebuild` | accepted in order; hub outbox empty; no live row; client state survives rebuild | — | `replica_exchange_7` `engine_serves_the_exchange_it_needs` + unit tests beside `materialize`/`bootstrap` |
| AC-18 | Coverage manifest and runner | checker exits 0; case runs the suites | Unknown case exits 2 | `bash scripts/check-replication-coverage.sh`; `bash scripts/test-replication-e2e.sh --case exchange` |
| AC-19 | A real `find` + `feedback --used` on A; A's captured `save`/`find`/`feedback` runs | B: one `feedback_events` row under A's event id, `used_count` 1, every shared run once; nothing owed | Fixed point after two more passes both ways; relayed id answered `duplicate` | `replica_exchange_8` `a_verdict_and_its_runs_reach_the_peer_and_count_once` + `drain/tests/adopt.rs` + `replica/tests/pulled.rs` `an_event_already_held_is_never_counted_again_under_another_id` |

Unit tests beside the modules cover the negotiation table exhaustively, the
`Retry-After` parser, the backoff window, the hold-reconsideration rule, the
pass-end rule and the pull entry classifier, the last over real engine
response bodies captured from a spawned engine inside the test (a `kind=`
page with gaps between its entries serves the `server_withheld` recorder).
The `server_withheld` path against a policy-filtering platform runs in the
platform harness with #183.

## Documentation impact

- This design is the contract; `docs/README.md` links it.
- `docs/guides/cloud-sync.md`: the drain, what each held state means and how it
  clears, retry/backoff and auth suspension, verify/rebootstrap, workspace
  switching, and repository-policy compatibility across old and new servers.
- `docs/configuration.md`: `[sync] request_timeout`, `[sync] pass_budget`,
  `[sync] max_request_bytes`.
- `docs/cli-reference.md`: the `status`/`verify` JSON fields (generator-owned
  sections regenerated).
- `docs/guides/replication-e2e.md`: case `exchange`.
- `docs/designs/2026-09-21-replica-v1-journal.md`,
  `2026-09-22-code-generation-replication.md`,
  `2026-09-23-document-revision-replication.md`,
  `2026-09-22-memory-mutation-capture.md` and
  `2026-09-24-feedback-activity-replication.md`: short notes where this
  issue changes their stated behavior (operation id on accept, `kind=` pages,
  capabilities, seeding, pending-refusal scope, the pull path's parent check,
  document enqueue, event adoption).
- `src/domains/sync/README.md`, new `src/domains/sync/drain/README.md`,
  `src/domains/sync/replica/README.md`, `src/store/README.md`.

## Open Questions

None blocking. Decisions taken without a human, with the reason:

1. **Policy for an unmanaged engine comes from the persisted per-key snapshot**
   (tests write it through the store API). A test proxy answering the policy
   route would be a canned HTTP result; approving whatever the machine can
   identify would weaken the platform's authority. Jev: snapshot 1.00 vs 0.00;
   canned status route judged a violation (0.93); identity-based approval judged
   a weakening (0.94).
2. **An unreadable-version entry stalls the pull** rather than being recorded
   and skipped: the issue forbids advancing past it (Jev 0.89). The stall ends
   the pass instead of spinning, and pushes continue.
3. **The old server in engine CI is a real engine that has not finished
   seeding**; the platform-specific old-server run belongs to the platform
   harness (Jev 0.71 for this plan vs 0.55 for a decision-table-only plan).
4. **The fault proxy may refuse with `429` or `502`, or corrupt one response
   byte**, never answer success (Jev 0.89 / 0.88).
5. **Legacy delivery is recognized by content, not by `pushed_seq`**, because
   the legacy push advances its cursor past rows it never delivered. A row whose
   bytes the upstream lacks is sent; the worst case is a content-identical
   second position, never a lost change.
6. **Re-publishing acknowledged data a restored upstream lost is #256**:
   re-sending old operations after a stream replacement could order them after
   newer writes the restored upstream kept.
7. **The request budget is a real setting, `[sync] max_request_bytes`**, not a
   constant: no real input in this repository reaches 4 MiB (this repository's
   whole-`src/` generation is 0.9 MiB), and an operator behind a proxy with a
   smaller body limit needs the same knob. Jev preferred it over padding the
   corpus with dependency sources (0.95 vs 0.05; "test-only knob" 0.10).
