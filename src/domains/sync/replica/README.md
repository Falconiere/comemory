# `domains/sync/replica/`

**What belongs here:** the `replica-v1` contract and the engine halves that
serve it — the versioned envelope both repositories implement, the acceptance
decision, the transaction that materializes state with its journal position and
receipt, the `changes` / `manifest` / `events` reads, staged revisions, and the
seeding that teaches the journal about memories older than itself.

**What does NOT belong here:** the legacy memory-only wire (that stays in
[`exchange/`](../exchange/README.md), byte-for-byte unchanged), any SQL (every
row read or written goes through [`store/`](../../../store/README.md)'s
`replica_*` modules), HTTP routing and envelopes (`serve/routes/sync_replica.rs`),
and the client half of push/pull — issues 251–255 switch `comemory sync` over.

The durable contract is
[docs/designs/2026-09-21-replica-v1-journal.md](../../../../docs/designs/2026-09-21-replica-v1-journal.md).

## Contents

| File | Primary item | Purpose |
| --- | --- | --- |
| `contract.rs` | `ImportRequest`, `Operation`, `Disposition` | The import envelope, its caps (500 operations, 5 MiB), the operation shape, and the typed dispositions — including `payload_erased` / `payload_expired`, kept separate from `rejected_invalid` so retention can drop bytes without stalling a bootstrap |
| `contract_views.rs` | `ChangesResponse`, `ManifestResponse` | The read and staging shapes: the ordered page and its `payload_state`, the per-kind manifest with its 256 bucket digests and capability list, and the stage/activate bodies |
| `validate.rs` | `decide`, `check_cursor` | The acceptance decision made before any state moves: kind and schema support, digest-covers-payload, content-derived identity, the erasure barrier, and tombstone ordering (an upsert cannot revive a deletion; a restore must name the one it observed) |
| `accept.rs` | `run`, `apply_one` | The import path: refuse the envelope this engine will not read at all, answer a replay from its receipt, persist a refusal so the replay reads the same answer, and hand an accepted operation to `materialize` |
| `materialize.rs` | `apply` | The write half — markdown first (it cannot join a SQLite transaction; recovery is issue 251), then one transaction carrying the mirror row, both journal feeds and the receipt |
| `code_accept.rs` | `handles`, `apply` | The write half for a code generation, which has no markdown: record it, replace the repo's pulled projection, activate it, journal the position and receipt it in ONE transaction, so the whole generation becomes visible at a single instant and a stale parent publishes nothing |
| `changes.rs` | `run` | The ordered page above a cursor, carrying the payload each position *named* rather than today's bytes |
| `manifest.rs` | `run` | Holdings, head, per-kind bucket digests, and the capability that stays empty until seeding completes |
| `events.rs` | `frames` | Notification-only frames (`sequence`, `entity_kind`) — enough to prompt a pull, never content |
| `staging.rs` | `stage`, `activate` | Parts of an oversized revision and their activation: invisible until every declared part has arrived, then accepted through the ordinary path. An incomplete upload keeps its parts for the retry; once acceptance has answered — accepted or refused alike — the parts are discarded, because a receipt is keyed on the bytes that arrived and the same operation id can never accept afterwards |
| `bootstrap.rs` | `advance`, `progress` | Seeding memories that predate the journal, 200 per replica call, restartable and idempotent, gating the advertised capability |

Tests live beside the modules under `tests/`, sharing `tests/support.rs` — a
real data directory, a real migrated database and real saved memories.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel.

An `Operation` carries an optional `vector` alongside its payload, never
inside it: the digest is taken over `payload` alone, so re-embedding a memory
cannot mint a new revision and two engines holding the same text agree on its
identity whether or not either has a vector. `validate::decide` refuses an
operation for an entity this machine still owes an unpushed change to
(`rejected_stale`) — the payload the outbox holds is the only record of that
edit. The manifest reports `needs_embedding`, the count of memories stored
without a usable vector. See
[the design](../../../../docs/designs/2026-09-22-memory-mutation-capture.md).
