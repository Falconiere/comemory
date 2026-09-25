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
and the client half of push/pull, which is [`drain/`](../drain/README.md) (#255).

The durable contract is
[docs/designs/2026-09-21-replica-v1-journal.md](../../../../docs/designs/2026-09-21-replica-v1-journal.md).

## Contents

| File | Primary item | Purpose |
| --- | --- | --- |
| `contract.rs` | `ImportRequest`, `Operation`, `Disposition` | The import envelope, its caps (500 operations, 5 MiB), the operation shape, and the typed dispositions — including `payload_erased` / `payload_expired`, kept separate from `rejected_invalid` so retention can drop bytes without stalling a bootstrap |
| `contract_views.rs` | `ChangesResponse`, `ManifestResponse` | The read and staging shapes: the ordered page and its `payload_state`, the per-kind manifest with its 256 bucket digests and capability list, and the stage/activate bodies |
| `validate.rs` | `decide`, `check_cursor` | The acceptance decision made before any state moves: kind and schema support, digest-covers-payload, content-derived identity, the erasure barrier, and tombstone ordering (an upsert cannot revive a deletion; a restore must name the one it observed). The refusal of an import over an owed upload applies only on an engine that is itself a `replica-v1` client; `kind_and_shape` is the part a pulled entry shares |
| `validate_events.rs` | `shape`, `known_event` | The rules only the two event kinds have (#254): upsert only, a digest with no payload is `payload_expired`, and an event already held is `duplicate` (same bytes, with its first position) or `rejected_conflict` (other bytes) whatever operation id carries it — which is what stops a reverse-sync echo counting twice |
| `accept.rs` | `run`, `apply_one` | The import path: refuse the envelope this engine will not read at all, answer a replay from its receipt, persist a refusal so the replay reads the same answer, and hand an accepted operation to `materialize` |
| `activity_payload.rs` | `ActivityEventV1`, `shared_keys` | One shared command run and the closed allowlist of what its summary carries per command; `sync.import` and every unlisted command stay local, and the receiver re-checks the same list |
| `materialize.rs` | `apply` | The write half — markdown first (it cannot join a SQLite transaction; recovery is issue 251), then one transaction carrying the mirror row, both journal feeds and the receipt, under the incoming operation id. `Order::Upstream` is the pull's order (no local ordering decision); the derived-graph refresh is left to the caller — once per envelope or page |
| `code_accept.rs` | `handles`, `apply` | The write half for a code generation, which has no markdown: record it, replace the repo's pulled projection, activate it, journal the position and receipt it in ONE transaction, so the whole generation becomes visible at a single instant and a stale parent publishes nothing. On the pull path it activates in the upstream's order instead |
| `document_accept.rs` | `handles`, `apply` | The write half for a document revision: replace the pulled cache for that one document, journal the position and receipt it in ONE transaction. That commit IS the activation — there is no staged state, and an upload still arriving never reaches here |
| `event_accept.rs` | `handles`, `apply` | The write half for a feedback or activity event: row, counter contribution (a verdict only), feed position (origin `sync`) and receipt in ONE transaction; the receiver's own `activity.enabled` / `activity.summaries` decide whether and how an imported run is shown |
| `changes.rs` | `run` | The ordered page above a cursor, carrying the payload each position *named* rather than today's bytes. `kind=` scans `limit` raw positions and returns `next_sequence` = the last scanned, so a filtered page still advances |
| `manifest.rs` | `run`, `advertised` | Holdings, head, per-kind bucket digests, and the capabilities — `replica-v1` then one `<kind>@<version>` per payload this engine reads — that stay empty until seeding completes |
| `pulled.rs` | `apply` | ONE entry pulled from an upstream, applied in the upstream's order (#255): a receipt replay answers `duplicate`, kind, shape and erasure are checked, nothing ordering-related is re-decided, and a refusal records no receipt. `Landing::Rewrite` journals it under a fresh id when a compacting replay rewrites local state |
| `events.rs` | `frames` | Notification-only frames (`sequence`, `entity_kind`) — enough to prompt a pull, never content |
| `staging.rs` | `stage`, `activate` | Parts of an oversized revision and their activation: invisible until every declared part has arrived, then accepted through the ordinary path. An incomplete upload keeps its parts for the retry; once acceptance has answered — accepted or refused alike — the parts are discarded, because a receipt is keyed on the bytes that arrived and the same operation id can never accept afterwards |
| `bootstrap.rs` | `advance`, `progress` | Seeding memories that predate the journal, 200 per replica call, restartable and idempotent, gating the advertised capability. Seeding journals without queueing an upload, and discards the outbox rows it would otherwise leave |
| `event_capture.rs` | `advance` | The activity capture sweep (every run recorded here, by an exact id cursor) and the one-time backfill of verdicts recorded before sharing existed — which never touches a counter — advanced beside the memory bootstrap and gating the capability with it |

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
