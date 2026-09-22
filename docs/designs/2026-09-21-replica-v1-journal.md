# Replica-v1 journal, receipts and server-ordered protocol — Design

**Date:** 2026-09-21   **Status:** Approved   **Author:** Auto
**Topic:** The engine's replication journal and the `replica-v1` wire contract (issue 250).

Parent epic: Falconiere/comemory#248. This issue ships the journal, the
contract and the engine routes. It changes no client push/pull policy: what
`comemory sync` sends today is unchanged.

## Problem

Replication was memory-shaped and reconstructive. `sync_log` is a memory-only
feed, `sync_state` a single cursor pair, and `GET /sync/changes` re-read the
*current* markdown for every historical row — so a feed entry described today's
bytes, not the bytes accepted at that position. There was no operation
identity, so a retry whose acknowledgement was lost looked like a new write; no
receipt, so an importer could not answer a replay with its original decision;
and a cursor was a bare integer, so a restored server backup read as "caught
up" instead of "your stream was replaced".

Issues 251–255 add code metadata, documents, feedback and activity. Each must
plug into one journal instead of growing a second memory-shaped feed.

## Non-Goals

1. No change to what `comemory sync` pushes or pulls. The client switch to
   `replica-v1` belongs to 251–255.
2. No new replicated entity kinds. `memory` is the only kind registered here.
3. No daemon, watch-loop or notification-delivery change (257, 258).
4. No platform work. `CodaSignal/comemory.io` issues 183–185 implement the
   server half against this document.
5. No repository-policy or authorization change beyond refusing a
   client-supplied workspace.
6. No removal of `sync_log`, `sync_state` or `sync_binding`, and no change to
   the JSON on the four legacy sync routes.
7. Embeddings are not replicated. The payload covers metadata and body; a
   re-embed must not read as a new revision. BYO vectors keep travelling on the
   legacy wire.

## Architecture

### One acceptance journal beside the legacy feed

`replica_feed` is the ordered acceptance record: one row per accepted mutation,
local or imported, written in the same transaction as the materialized state.
Memory mutations also keep appending their `sync_log` row in that transaction
(`domains::memories::journal` writes both), so the two feeds cannot describe
different histories and legacy history survives.

The trade-off: a second table rather than widening `sync_log`. Widening would
force every legacy reader — and the platform's decoder — through a schema
change, and would leave `memory_id` / `content_hash` as the shape every future
entity kind must wear.

### Immutable payloads, content-addressed

`replica_payload` stores canonical payload bytes keyed by their SHA-256 digest.
A feed row references a digest; it never points at live state. Canonicalization
is `utilities::canonical_json` (object keys sorted at every depth, no
insignificant whitespace), so two machines that build the same payload in
different key orders agree on its digest.

The digest covers the whole payload object, so a tags-only or references-only
edit is a new revision even though the body hash is unchanged.

Permanent erasure blanks `bytes` and stamps `redacted_at`. The row — and its
digest — stay: that is the barrier that stops an old upsert from resurrecting
erased content, and it is reported as the typed disposition `payload_erased`
rather than as a validation failure.

### Payload ownership

`MemoryPayloadV1` (`domains::memories::replica_payload`) is the memory
payload: the replicated frontmatter minus `author`, plus the body, flat. The
accepting side stamps authorship, so a peer cannot claim one. `domains::sync`
consumes the type through the existing `sync → memories` dependency; nothing in
`domains::memories` reaches into `domains::sync` — it writes through
`store::replica_journal`, which takes passive rows only.

### Operation identity and receipts

Every mutation gets an `operation_id` — `utilities::dated_id` under an `op-`
prefix (`op-<yyyymmdd>-<8hex>`), seeded with the entity and nanosecond clock.
Locally it is stored in `replica_operation`, the durable outbox, in the same
transaction as the mirror write. When this engine accepts an operation from a
peer it writes `replica_receipt` in the same transaction as the materialized
state and the feed row.

A replay of the same `operation_id` is answered from its receipt, including its
original `sequence` — never a newer position. The same `operation_id` with a
different `payload_digest` is `rejected_conflict`; nothing is materialized.

### Server acceptance ordering

`replica_feed.sequence` is `INTEGER PRIMARY KEY AUTOINCREMENT`, assigned by the
accepting engine and never carried from a client. `replica_revision` holds one
row per entity — its current sequence, digest, and deletion state including the
deletion's own position.

- An upsert against a tombstoned entity is `rejected_stale`.
- A restore must carry `observed_sequence` equal to the deletion's sequence;
  otherwise `rejected_stale`. A blind restore is an overwrite, not a restore.
- A tombstone for an id this engine never held is accepted and recorded: the
  peer's deletion is a fact about the shared stream, and recording it is what
  stops a later pull from re-creating what was deleted.

Content-derived memory ids keep their existing identity and collision rules.

### Epoch-bearing cursors

`replica_stream` holds this database's epoch — 32 hex characters minted by
migration 0022's post-apply pass from `/dev/urandom`. `replica_cursor` holds,
per workspace, the upstream epoch and applied sequence. Every request may carry
an epoch; a mismatch is HTTP `409 epoch_mismatch`, never an empty page. A
cursor *at* the head reads as caught up — an empty page with the real
`head_sequence` — while a cursor *above* it names a position this stream never
issued and is refused with `409 cursor_ahead`, so a peer holding progress from
somewhere else cannot keep believing it is up to date.

A `comemory rebuild` copies the epoch, the feed, the revisions, the receipts
and the outbox (`store::rebuild_copy_history`): a rebuild is not a replaced
stream, and a sequence already handed to a peer cannot be reissued. Staged
parts are not copied — an upload that never activated published nothing.

### Bootstrap seeding

Memories saved before the journal are seeded by
`domains::sync::replica::bootstrap`, in batches of 200 in ascending id order,
advanced by the replica cores themselves on each `changes` / `manifest` /
`import` call. No scheduler and no daemon. Progress lives in `schema_meta`
(`replica_bootstrap_state`, `replica_bootstrap_through`); seeding is restartable
and idempotent, and a concurrent save appends at the head like any other
mutation. `manifest.capabilities` stays empty until the state is `complete`,
so a peer never treats a half-seeded feed as the whole history.

### Oversized revisions stage before they activate

An envelope carries at most 500 operations and 5 MiB. A larger single revision
is uploaded to `POST /sync/replica/stage` in indexed parts and published by
`POST /sync/replica/activate`, which assembles them and runs the ordinary
acceptance path. Parts live in a table nothing else reads, so an interrupted
upload is invisible to `changes` and `manifest`; a missing part is
`409 staging_incomplete`.

## Interfaces / Schema

### Migration `0022_replica_journal` (additive)

| Table | Key | Holds |
| --- | --- | --- |
| `replica_stream` | `id = 1` | This database's stream epoch |
| `replica_payload` | `digest` | Canonical bytes, byte length, erasure stamp |
| `replica_feed` | `sequence` (autoincrement) | Ordered acceptances; unique on `operation_id` |
| `replica_revision` | `(entity_kind, entity_key)` | Current sequence, digest, deletion state |
| `replica_operation` | `operation_id` | The outbox: state, attempts, upstream answer |
| `replica_receipt` | `operation_id` | The decision this engine gave |
| `replica_cursor` | `workspace_id` | Upstream epoch and applied sequence |
| `replica_staged_part` | `(staging_id, part_index)` | Parts of an oversized revision |

All eight are declared in `store::schema_replica` and generated by
`just migration`; every SQL string stays inside `store/`.

### Wire contract `replica-v1`

`POST /api/v1/sync/replica/import`:

```json
{
  "protocol": "replica-v1",
  "cursor": { "stream_epoch": "…32 hex…", "sequence": 412 },
  "operations": [
    {
      "operation_id": "op-20260921-9f2c4ab1",
      "entity_kind": "memory",
      "entity_key": "a1b2c3d4",
      "op": "upsert",
      "schema_version": 1,
      "payload_digest": "…64 hex…",
      "observed_sequence": 411,
      "repository": "Falconiere/comemory",
      "payload": { "id": "a1b2c3d4", "kind": "decision", "body": "…", "…": "…" }
    }
  ]
}
```

```json
{
  "protocol": "replica-v1",
  "stream_epoch": "…32 hex…",
  "head_sequence": 413,
  "results": [
    { "operation_id": "op-20260921-9f2c4ab1", "disposition": "accepted", "sequence": 413,
      "payload_digest": "…64 hex…" }
  ]
}
```

A body that carries `workspace_id` is refused with `400`: workspace comes from
the authenticated credential, never from the request.

Dispositions: `accepted`, `duplicate`, `rejected_stale`, `rejected_conflict`,
`rejected_unsupported`, `rejected_invalid`, `rejected_not_allowed`,
`payload_expired`, `payload_erased`. The last two are typed separately from
`rejected_invalid` so retention can drop historical bytes without stalling a
valid bootstrap.

`GET /api/v1/sync/replica/changes?since=&limit=&kind=&epoch=` returns
`{ protocol, stream_epoch, head_sequence, next_sequence, entries }`, where each
entry carries `payload_state` (`present` | `absent` | `erased`) and the payload
accepted at that position.

`GET /api/v1/sync/replica/manifest` returns the stream, the head,
`capabilities`, one `entity_kinds[]` entry per kind (schema version, live
count, and 256 bucket digests keyed by the digest's first two hex characters —
the same bucketing the legacy manifest uses) and `bootstrap { state, seeded }`.

`GET /api/v1/sync/replica/events?since=&epoch=` returns notification-only
frames `{ sequence, entity_kind }`. No content crosses this feed; it is
resumable from any position because it is read from the journal.

`POST /api/v1/sync/replica/stage` takes
`{ protocol, staging_id, part_index, part_count, bytes }`;
`POST /api/v1/sync/replica/activate` takes `{ protocol, staging_id, operation }`
where the operation's `payload` is absent.

### Route classes

| Route | Class |
| --- | --- |
| `GET /sync/replica/changes` | read |
| `GET /sync/replica/manifest` | read |
| `GET /sync/replica/events` | read |
| `POST /sync/replica/import` | mutating |
| `POST /sync/replica/stage` | mutating |
| `POST /sync/replica/activate` | mutating |

No replica route starts or repairs a daemon: this is the hosted engine's
request path, not its lifecycle.

## Failure modes

| Input / failure | Behavior |
| --- | --- |
| Replay after a lost acknowledgement | `duplicate` with the original sequence |
| Reused id, different bytes | `rejected_conflict`; nothing materialized |
| Unknown kind or schema version | `rejected_unsupported`; cursor unmoved |
| Digest disagrees with payload, or body no longer hashes to its id | `rejected_invalid` |
| Upsert over a tombstone | `rejected_stale` |
| Restore naming the wrong deletion | `rejected_stale` |
| Foreign epoch on `changes` / `import` / `events` | `409 epoch_mismatch` |
| `since` at head, matching epoch | Empty entries, real `head_sequence` |
| `since` above head | `409 cursor_ahead` |
| 501 operations or a >5 MiB envelope | `400`; nothing applied |
| Missing staged part | `409 staging_incomplete`; nothing published |
| Erased payload resent | `payload_erased`; barrier stands |
| Crash mid-import | The database has all three rows (state, position, receipt) or none; markdown recovery is issue 251 |
| `comemory rebuild` | Sequences, receipts, revisions and epoch preserved |

## Evidence

`tests/replica_contract.rs` and `tests/replica_contract_2.rs` drive real
spawned `comemory serve` processes, a real CLI, real HTTP and the real SQLite
databases on both sides. `bash scripts/test-replication-e2e.sh --case contract`
runs them; `scripts/replication/coverage.json` maps `F-1`…`F-8` to that case.
