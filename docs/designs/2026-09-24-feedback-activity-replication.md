# Replicate feedback and activity as events, counted exactly once — Design

**Date:** 2026-09-24   **Status:** Approved   **Author:** Falconiere R. Barbosa
**Topic:** Feedback verdicts and activity rows as two `replica-v1` entity kinds (issue 254).

Issue: [#254](https://github.com/Falconiere/comemory/issues/254). Parent epic:
[#248](https://github.com/Falconiere/comemory/issues/248). Builds on
[the replica-v1 journal](2026-09-21-replica-v1-journal.md) (#250),
[memory mutation capture](2026-09-22-memory-mutation-capture.md) (#251),
[code generation replication](2026-09-22-code-generation-replication.md) (#252)
and [document revision replication](2026-09-23-document-revision-replication.md)
(#253). The activity feed itself is [2026-09-20-activity-feed.md](2026-09-20-activity-feed.md).

## Problem

Feedback and activity are the two local histories that change ranking and tell
a team what its agents are doing, and neither can be copied as rows.

- `feedback_events.id` and `activity_log.id` are local integers. Two machines
  both hold a row `42`, and they are different events.
- A verdict is two writes: an event row and a counter bump in `feedback` or
  `code_feedback`. Replaying the rows, or the counters, counts a verdict once
  per replay. Ranking then rewards whatever was synced most often.
- A code verdict names a `code_symbols` rowid. Re-indexing recycles rowids, so
  the number means nothing on another machine and may mean a different symbol
  here a week later.
- An activity summary holds free text — a query, a title — and the `repo` it
  was scoped to may be one no workspace approved.
- Importing activity is itself activity (`sync.import`). A feed that shared
  everything would share its own imports back forever.

Issues 250–253 gave memories, code generations and documents a journal
position, a receipt and an identity that means the same thing everywhere. This
does the same for verdicts and command runs.

## Non-Goals

1. No client push/pull loop. Draining the feed to the platform and applying a
   pulled page is #255. This issue owns the two entity kinds: identity,
   payload, capture, acceptance, retention and readers — the shape #253 owned
   for documents. Nothing is enqueued on `replica_operation`; the feed position
   is the durable record a push reads.
2. No platform work. Relay, fan-out and console views are
   CodaSignal/comemory.io #183–#185. The console's read visibility for shared
   activity is coordinated with #185, not decided here. Platform learning-job
   authorization (comemory.io #153) is not touched.
3. No replication of `retrieval_log`, retrieval passages, candidate
   observations or judgments, bandit arms, query expansions, eval history or
   session transcripts. They stay local.
4. No aggregate counter replication. Counters are never sent; only events are,
   and each receiver derives its own counter contribution from them.
5. No new CLI command or flag. `GET /api/v1/activity` gains one field; the
   replica routes gain one payload state.
6. No change to what a verdict or an activity row means locally. Local counters,
   local provenance, local recording and the `activity.*` config keys behave as
   before.
7. Rebuild/restore semantics beyond copying the new rows are #256.

## Architecture

### Two entity kinds, both immutable events

| Kind | `entity_key` | One entity is | Schema |
| --- | --- | --- | --- |
| `feedback_event` | the event id | one verdict on one target | `FeedbackEventV1`, version 1 |
| `activity_event` | the event id | one recorded command run | `ActivityEventV1`, version 1 |

An event never changes after it is recorded, so both kinds accept `upsert` only.
A `tombstone` or `restore` for either kind is `rejected_invalid`. Retention and
purge remove bytes through payload redaction (below), never through a tombstone,
so a receiver's counters are not asked to "undo" a verdict.

### Identity: an event id and a device id

**Event id.** `ev-` followed by 32 lowercase hex characters from
`store::random_id::random_hex(16)`. It is minted once, when the event is
journalled, and stored on the row (`feedback_events.event_id`,
`activity_log.event_id`, each under a unique index). Random rather than derived
from the local rowid: `feedback_events.id` is a plain `INTEGER PRIMARY KEY`
whose rowid SQLite reuses after the highest row is deleted, and a reused number
would mint the id of an event a peer already counted.

**Device id.** Each database mints one 32-hex id in a new single-row table,
`replica_device`, minted by migration 0026 itself from SQLite's randomness
source (the OS's, through the unix VFS); the migration runs once per database.
It is not the epoch: the epoch changes when a stream
is replaced or restored (#256), while the device that recorded an event does
not. Every payload carries the recording device. A receiver stores the sender's
device on the imported row, so an imported event is never attributed to the
machine that received it. `NULL` in a `device` column means "recorded here".

**Namespaced query id.** A verdict cites the query it answered as
`<device>:<query_id>` (`origin_query_id`). That value can never equal a local
`q-<yyyymmdd>-<8hex>`, so an imported verdict cannot join this machine's
`retrieval_log`, cannot clear a local pending recall, and needs no query row to
be invented. The originating `retrieval_log` row can be evicted before the
verdict travels. The verdict does not depend on it.

### Canonical targets

| Target | Wire identity | Version |
| --- | --- | --- |
| memory | `{"kind":"memory","id":"<8-hex memory id>"}` | none: the id is content-derived |
| code | `{"kind":"code","repo":"<canonical>","path":"<repo-relative>","symbol":"<name>"}` | `version`: the symbol row's `blob_oid`, when the row has one |

A code verdict is resolved to `(repo label, path, symbol)` at record time by the
existing chunk-to-parent walk (`learning::code_feedback::resolve_identity`,
widened from private to `pub(crate)` so `feedback_share` can call it), because
that is the only moment the rowid is known to mean that symbol. The walk also
returns the resolved row's `blob_oid` (`store::code_feedback::own_identity` /
`parent_identity` read one more column). The
label is then mapped to its canonical repository through `repository_approval`.

On acceptance, a code verdict's counter is keyed by this machine's label for
that canonical repository (`repository_approval::label_for`, the smallest
matching label). With no approved label — the hosted engine's case — it is keyed
by the canonical name itself. Keying by the local label is what lets the
existing `code_prior` join, which matches `code_feedback.repo` against
`code_symbols.repo`, see the imported verdict.

### What is shared, and what stays local

An event is **shared** only when every rule holds. Otherwise it is recorded,
counted and shown locally exactly as today, gets no event id, and never leaves
the machine.

**Feedback verdicts**

| Rule | Why |
| --- | --- |
| Provenance is `manual`, `implicit` or `auto_search_edit` | These are observations of what a caller or an agent on this machine did. |
| Provenance is not `auto_coactivation` | Every machine that indexes a repository mines the same commits and mints its own co-activation reward, so sharing it would count one commit once per machine. |
| The target resolves to an approved repository | Memory target: the memory's `repo` label (`memory_repository::label`). Code target: the symbol's label. Mapped with `repository_approval::canonical_for`. |
| No local repository label, or none approved | Unscoped. Stays local. |

**Activity rows**

| Rule | Why |
| --- | --- |
| `device IS NULL` | An imported row is never re-captured. This is the loop guard. |
| `command` is in the allowlist below | `sync.import` and any command not listed is bookkeeping or unreviewed. |
| `repo` is set and approved | An unscoped run (`repo` NULL) is multi-repo or unscoped by definition. |

**Shared summary allowlist.** A shared activity event carries only these keys
from the local summary. Every other key is dropped, even if a future core adds
one. The receiver re-checks the same allowlist and refuses a payload carrying
anything else (`rejected_invalid`).

| `command` | Shared keys |
| --- | --- |
| `save` | `id`, `kind`, `tags`, `supersedes` |
| `update` | `id`, `fields` |
| `delete`, `restore` | `id` |
| `search`, `context` | `query`†, `hits` |
| `find` | `query`†, `hits`, `total` |
| `search-code` | `query`†, `hits`, `lang` |
| `feedback` | `used`, `irrelevant`, `used_code`, `irrelevant_code`, `provenance` |
| `index-code` | `files`, `mode` |

Dropped deliberately: `title` (memory content that purge policy must be able to
reach), `query_id` (a local id), `top` and `targets` (can name local code rowids
and document ids), `known_query` and `derived_stale` (facts about this machine's
tables), and the summary's own `repo` (the event carries the canonical repo).

† **Free text** — `query`, and the event's `actor` label on both kinds — goes
through one policy, `utilities::shared_text`:

1. The raw text is scanned with the curated `utilities::secret_scan` rule set.
   A match anywhere — inside a path token included — withholds the whole
   field: `query` is omitted and `"query_withheld": true` is set; `actor`
   becomes `null`. The event is still shared. A withheld value is never
   logged, including at debug level.
2. In text that may leave, every whitespace-separated token that is an
   absolute machine path — `/` followed by at least two segments, `~/…`, or a
   drive-letter path — becomes `<path>`.

`activity.enabled = false` records no row, so there is nothing to share.
`activity.summaries = false` records `summary = NULL`, so the shared event
carries `summary: null`. Capture never builds a summary the local row did not
store.

### Capture

**Verdicts are journalled at record time**, inside the transaction that already
writes the event row and bumps its counter (`learning::feedback::apply`, and
`record_implicit_used` for `auto_search_edit`). Two reasons. A code target's
identity is only resolvable at that instant. And the documents rule from #253
holds: a verdict that exists locally but owes no feed position is the
divergence the journal exists to prevent. A failure aborts the whole verdict
batch, as a failed code identity already does.

The same transaction now also records the caller's delivery `surface` (`cli`,
`http`, `mcp`) and declared `actor`, taken from `Ctx::origin`. Local rows were
missing both, and the payload must preserve them. Every verdict writer goes
through the one capture helper, so none can write a shareable verdict without
its feed position: `feedback::run` passes its origin; the public
`record_with_provenance` / `record_code_with_provenance` and the
`auto_search_edit` reward (`record_implicit_used`, inside `graph::materialize`'s
transaction) have no caller origin and record `surface` and `actor` as `NULL`.

**Activity rows are journalled by a sweep**,
`domains::sync::replica::event_capture::advance`. It reads `activity_log` rows
above a `schema_meta` cursor (`replica_activity_capture_through`) in ascending
id order, 200 per call. It journals the shareable ones, stamps their event id,
and advances the cursor, all in one transaction per batch. Recording stays
exactly as best-effort as the activity design requires: a capture failure is
retried by the next sweep and can never fail the command that ran.
`activity_log.id` is `AUTOINCREMENT`, so the cursor never skips or revisits a
row. The same sweep therefore covers rows recorded before this feature and rows
recorded after it.

**Retained legacy verdicts are backfilled by the same sweep.** It makes one pass
over `feedback_events` rows with `event_id IS NULL AND device IS NULL`,
memory-target, and a shareable provenance, cursor
`replica_feedback_backfill_through`. The pass ends in state `complete` and never
runs again; from then on, record-time capture is the only path. Backfill:

- journals the event and stamps its id, and **never touches a counter**. This
  machine's counters already include every retained and every expired event.
- is idempotent. A stamped row is skipped, so a second pass journals nothing.
- skips legacy **code** verdicts. Their row holds only a rowid that re-indexing
  may since have given to another symbol, and sharing a verdict on the wrong
  symbol is worse than not sharing it. They stay local.
- cannot see an expired event, because gc already deleted its row. That event's
  contribution stays in this machine's counter and is never exported.

`advance` runs where `bootstrap::advance` runs — in the `changes` and
`manifest` cores, right after it — and `manifest.capabilities` stays empty
until both the memory bootstrap and this backfill are `complete`. The
`bootstrap.state` wire field keeps its three values and merges the two state
machines by one rule: while the memory bootstrap is `pending` or `seeding`, it
reports that; once the memory bootstrap is `complete`, it reports `seeding`
until the feedback backfill is `complete` too, and `complete` after. The
activity sweep has no terminal state (it follows the table forever), so it does
not gate the capability.

### Acceptance: one transaction per event, guarded twice

Every accepted event writes, in one transaction: its materialized row, its
counter contribution (verdicts only), its feed position (origin `sync`, never
enqueued) and its receipt. SQLite commits all four or none. A process killed
mid-envelope leaves each operation fully applied or absent.

Two guards stop a second effect:

1. **Operation receipt** (existing). The same `operation_id` is answered from
   its receipt.
2. **Event revision** (new, events only). `replica_revision` already holds one
   row per `(entity_kind, entity_key)`. An event whose key already has a
   revision is not applied again, whatever operation id carries it:
   - payload expired or erased here: `payload_expired` / `payload_erased`;
   - otherwise same digest: `duplicate`, with the revision's original
     `sequence`;
   - otherwise: `rejected_conflict` (the same event id cannot name two events).

   The existing `order()` accepts any upsert over a live revision, which is
   right for memories and wrong for events, so this guard is new logic, in
   `validate_events::known_event`. It returns the disposition and, for
   `duplicate`, the sequence. `accept::record_refusal` gains a
   `sequence: Option<i64>` parameter (every existing caller passes `None`), so a
   `duplicate` receipt stores the original sequence and a later replay of that
   operation id reads it back.

**Where the event rules sit in `validate::decide`.** `kind_and_shape` routes
the two event kinds to `validate_events::shape` instead of the shared `shape`:
it refuses a `tombstone` or `restore` (`rejected_invalid`), answers an `upsert`
carrying a `payload_digest` but no `payload` with `payload_expired`, and
otherwise runs the shared digest check and the kind's `Identity` rule. Memory,
code and document operations take exactly the path they take today. After the
shape check, `decide` reads the payload's redaction (below) and then, for event
kinds only, runs `known_event` before `order`.

The second guard catches the reverse-sync echo. Machine A records `E`; B imports
it; B's feed is relayed back and A is offered `E` under A's own operation id,
for which A holds no receipt because A never imported it. A's local revision for
`E` answers `duplicate`, and A's counter does not move. Two genuinely separate
`comemory feedback` calls mint two event ids and both count everywhere.

Receiver-side materialization:

| Kind | Row written | Counter |
| --- | --- | --- |
| `feedback_event` | `feedback_events`: `query_id = origin_query_id`, `memory_id =` memory id or `<repo>//<path>#<symbol>`, the payload's verdict, `at`, provenance, surface, actor, `device`, `event_id` | memory: `feedback` by id; code: `code_feedback` by (local label or canonical, path, symbol). `used` moves `last_used` to `MAX(last_used, at)`, so an old verdict cannot move it backwards. |
| `activity_event` | `activity_log` with the original `at`, `source`, `actor`, `duration_ms`, `ok`, `error_code`, `repo` = local label or canonical, `device`, `event_id`; `summary` = the shared summary, or `NULL` when this machine has `activity.summaries = false`; no row at all when it has `activity.enabled = false` | none |

A receiver with activity disabled still writes the feed position and receipt,
so convergence and deduplication do not depend on local display settings. The
journal keeps the accepted bytes, as it does for every kind.

An accepted import records no activity row. The legacy `sync.import` row is
never shareable. An imported row carries a `device`, so the capture sweep skips
it. Those three rules together mean applying activity cannot generate activity
to share.

### Retention and purge reach the journal

`comemory gc` already evicts `feedback_events` and `activity_log` rows older than
`prune.learning_retention_days`. It now also **expires** the journal copies,
before those rows are deleted: the payload bytes of every event are blanked and
`replica_payload.redaction` is set to `expired` when either the event's
materialized row is about to be evicted (its `event_id`, read before the
delete), or its `replica_feed` row (an event kind) has `at` before the same
cutoff. The second arm reaches imported events a receiver never materialized
(activity disabled there). `store::gc_learning::evict_before` is extended to do
the expiry for both kinds in its own transaction: it reads the `event_id`s of
the `feedback_events` AND `activity_log` rows below the cutoff, plus the
event-kind feed rows below it, expires those payloads, then deletes the
`retrieval_log` and `feedback_events` rows as today. `maintenance::gc` already
calls it before `activity::delete_before`, so by the time the activity rows are
deleted their journal copies are already expired, and `maintenance::gc` gains
no code. What stays is the dedupe and contribution
metadata — the feed row, the revision, the digest and the receipt — so a later
offer of that event is recognized and answered `payload_expired`, not counted
again. `changes` renders such an entry as `payload_state: "expired"` with its
digest and no payload.

`payload_expired` is also the typed answer for an event-kind upsert that
carries a `payload_digest` but no `payload`: what a pulled entry looks like
after its origin's retention expired it. It is receipted, writes no feed
position and adds no contribution. A bootstrap relaying expired history
therefore advances past it instead of stalling on `rejected_invalid`.

`store::memory_purge::purge_memory` already deletes a purged memory's counter
row and its memory-target `feedback_events`. In the same transaction, and
before that delete, it now **erases** (`redaction = erased`) the journal
copies of those rows' events, in one statement keyed by the memory.
Replaying one of them answers `payload_erased` and restores neither row nor
counter. Shared activity carries no memory title, so purge policy has no
activity text to reach.

Existing redaction (`redact_payload`, permanent erasure) sets
`redaction = erased`. A row redacted before this migration has `redaction NULL`
and `redacted_at` set, and reads as erased.

Every reader of that state becomes three-way. `store::replica_read` replaces
`FeedRow.payload_erased: bool` with `redaction: Option<Redaction>` and
`is_erased(digest)` with `redaction_of(digest)`; `validate::decide` maps
`Erased` to `payload_erased` and `Expired` to `payload_expired`; `changes`
renders `erased`, `expired`, `present` or `absent`.

### Ordering

`replica_feed.sequence` orders every shared event, and `activity_log.id` — the
activity stream's SSE cursor — is assigned at acceptance. Both follow the order
the engine accepted events, not their `at`. Two events with the same timestamp
are therefore neither reordered nor skipped by a cursor. The newest-first
snapshot already breaks `at` ties by `id DESC`.

### Readers

| Reader | Change |
| --- | --- |
| `eval` golden harvest (`store::feedback::used_events_for_golden`) | `AND e.device IS NULL`: no replicated verdict, of any provenance, becomes ground truth |
| `mine` (`used_query_ids`) | `AND device IS NULL` |
| `recall-status` (`feedback::events_since`) | counts local verdicts only; imports create no query rows, so `queries` and `pending` are untouched too |
| `GET /api/v1/activity` + `/activity/events` | each item gains `device` (`null` = recorded here) |
| `learning/summary` event tiles | unchanged: they count every stored verdict, imported ones included |
| `code_prior` / memory rerank | unchanged: they read counters, which now include imported contributions |

## Interfaces / Schema

### Migration `0026_replica_events`

Generated with `just migration replica_events`, plus two hand edits: the two
`CHECK`s inlined on their `ADD COLUMN` (the generator only comments them), and
the one `INSERT` that mints the device id.

| Table | Change |
| --- | --- |
| `replica_device` (new) | `id INTEGER PRIMARY KEY CHECK (id = 1)`, `device_id TEXT NOT NULL`, `created_at TEXT NOT NULL` |
| `feedback_events` | `+ event_id TEXT`, `+ device TEXT`, `+ surface TEXT CHECK (surface IN ('cli','http','mcp'))`, `+ actor TEXT`; `UNIQUE INDEX uq_feedback_events_event_id (event_id)` |
| `activity_log` | `+ event_id TEXT`, `+ device TEXT`; `UNIQUE INDEX uq_activity_log_event_id (event_id)` |
| `replica_payload` | `+ redaction TEXT CHECK (redaction IN ('erased','expired'))` |
| `replica_feed` | `INDEX idx_replica_feed_kind_at (entity_kind, at)` — the feed-`at` arm of retention expiry reads only positions past the cutoff instead of every event position ever accepted |

**Rebuild.** The copy passes name their columns, so each new column is added
by hand:

- `store::rebuild_copy_learning_events` — the `feedback_events` copy gains
  `event_id, device, surface, actor` (as `NULL` from a pre-0026 source).
- `store::rebuild_copy_history` — the `replica_payload` copy gains `redaction`,
  and a `replica_device` copy is added; `replica_device` joins `COPIED_TABLES`.
- **`activity_log` has no copy pass today.** It is listed in `COPIED_TABLES`,
  and the coverage test checks only that listing, so every `comemory rebuild`
  silently drops the activity history. This issue adds the pass (with
  `event_id`, `device`), because a dropped event id would let an echoed event
  count twice. The coverage test's blind spot is otherwise left to #256.

### `FeedbackEventV1` (`domains::learning::replica_payload`)

```json
{
  "event_id": "ev-4f0c…(32 hex)",
  "device": "9a1e…(32 hex)",
  "at": "2026-09-24T10:00:00.123456789Z",
  "verdict": "used",
  "provenance": "manual",
  "surface": "mcp",
  "actor": "claude-code/2.1.0",
  "origin_query_id": "9a1e…:q-20260924-1a2b3c4d",
  "target": { "kind": "code", "repo": "Falconiere/comemory",
              "path": "src/store/feedback.rs", "symbol": "upsert_used",
              "version": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391" }
}
```

It is valid only when: `entity_key == event_id`; both ids have their shape;
`verdict ∈ {used, irrelevant}`; `provenance ∈ {manual, implicit,
auto_search_edit}`; `surface` is `null` or one of the three; `at` parses;
`origin_query_id` starts with `device + ":"`; a memory target has a valid
memory id; a code target has a non-empty repo, path and symbol. The operation's
`repository` is the canonical repository.

### `ActivityEventV1` (`domains::sync::replica::activity_payload`)

```json
{
  "event_id": "ev-…", "device": "…", "at": "…",
  "command": "find", "source": "mcp", "actor": "claude-code/2.1.0",
  "repo": "Falconiere/comemory", "duration_ms": 37, "ok": true,
  "error_code": null,
  "summary": { "query": "replica receipts", "hits": { "memory": 3, "code": 1, "document": 0 }, "total": 4 }
}
```

It is valid only when `entity_key == event_id`, `command` is allowlisted,
`source ∈ {cli, http, mcp}`, `repo` is non-empty, `duration_ms >= 0`, `at`
parses, and `summary` is `null` or an object whose keys are a subset of that
command's allowlist plus `query_withheld`.

### Wire additions (`replica-v1`, additive)

- `entity_kind` values `feedback_event`, `activity_event`, both
  `schema_version: 1`.
- `payload_state` gains `expired`.
- `payload_expired` and `payload_erased` answers, and `duplicate` for an event
  known under another operation id, now occur on import as described above.
- `GET /api/v1/activity` items: `+ "device": string | null`.

### New and changed modules

| Module | Holds |
| --- | --- |
| `store/schema_replica_device.rs` | `ReplicaDevice` |
| `store/replica_device.rs` | `id(conn) -> Result<String>` |
| `store/replica_redaction.rs` | `redact(conn, Reach, at) -> Result<u64>` — `Reach::PastRetention(cutoff)` expires, `Reach::VerdictsOn(memory_id)` erases, one statement however many events — and `redaction_of(conn, digest) -> Result<Option<Redaction>>` |
| `store/replica_read.rs` | `Redaction`; `FeedRow.redaction`; `redaction_of` (replaces `is_erased`) |
| `store/replica_journal.rs` | `redact_payload` also sets `redaction = erased`; `append_local_event` — the one local-upsert journal append both captures use |
| `store/feedback.rs`, `store/code_feedback.rs` | `NewFeedbackEvent` (one struct for every insert); `last_used = MAX(...)`; `device IS NULL` on the harvest/mine/recall readers; `own_identity`/`parent_identity` return `blob_oid` |
| `store/feedback_share.rs` (new, keeps `feedback.rs` under 300) | `stamp_event_id`, `unshared_legacy_after` |
| `store/activity.rs`, `domains/maintenance/activity.rs` | `device` on `ActivityRow` and on the API `Item`; `event_id`/`device` on insert |
| `store/activity_share.rs` (new, keeps `activity.rs` under 300) | `capture_batch_after`, `stamp_event_id` |
| `store/gc_learning.rs` | `evict_before` expires both event kinds' journal copies (read ids from `feedback_events` and `activity_log`, plus the feed-`at` arm) before its deletes, in one transaction |
| `store/memory_purge.rs` | erasure of a purged memory's verdict journal copies before its delete |
| `store/rebuild_copy_learning_events.rs`, `store/rebuild_copy_history.rs` | the column lists and the new `activity_log` / `replica_device` passes |
| `store/repository_approval.rs` | `label_for(conn, canonical)` |
| `utilities/shared_text.rs` | `for_share(raw) -> Shared { Kept(String), Withheld }`, `label_for_share(raw) -> Option<String>` |
| `utilities/telemetry.rs` | `entity::{FEEDBACK_EVENT, ACTIVITY_EVENT}` — the kind names `store` must also name |
| `cli/feedback.rs` | reads the layered config (`cli::load_config`) instead of `Config::defaults()`, so `COMEMORY_ACTOR` and `activity.*` reach a CLI verdict; it still creates nothing before validation |
| `domains/learning/replica_payload.rs` | `FeedbackEventV1`, `FEEDBACK_ENTITY_KIND` |
| `domains/learning/feedback_share.rs` | `journal` (the record-time journal of one verdict), `journal_retained` (the backfill's unit), `Caller` (surface + actor from `Ctx::origin`) |
| `domains/sync/replica/activity_payload.rs` | `ActivityEventV1`, `ACTIVITY_ENTITY_KIND`, the allowlist |
| `domains/sync/replica/event_capture.rs` | `advance` (activity sweep + feedback backfill) |
| `domains/sync/replica/event_accept.rs` | `handles`, `apply` for both kinds |
| `domains/sync/replica/validate_events.rs` (new; `validate.rs` is at 288 lines) | the two `Identity` impls, `shape` for event kinds, `known_event` |
| `domains/sync/replica/{validate,accept,changes,manifest}.rs` | routing to `validate_events`, the three-way redaction, `record_refusal`'s sequence, `expired`, capability gating |

`scripts/architecture-policy.json` gains `domains::sync -> domains::learning`
(acceptance decodes the verdict payload and writes counters), the same
direction `sync -> documents` already takes for documents.

## Failure modes and edge cases

| Input / failure | Behavior |
| --- | --- |
| Replay of an accepted event envelope | `duplicate`, original sequence; counters unchanged |
| Same event under a new operation id (echo, relay, re-backfill) | `duplicate` from the event revision; counters unchanged |
| Same event id, different bytes | `rejected_conflict`; nothing written |
| Two separate verdict calls, same query and target | Two event ids; both count on every machine |
| Process killed mid-envelope | Each operation's row, counter, feed position and receipt are all present or all absent |
| Counter write fails inside acceptance (induced) | Transaction rolls back; no event row, no position, no receipt; the retry applies once |
| Counter or journal write fails during local `feedback` | The whole verdict batch rolls back, as today |
| Originating `retrieval_log` row gc'd before the verdict travels | Verdict still shared with its namespaced query id and provenance; no query row created on the receiver |
| Materialized event row evicted by gc | Its journal copy is expired first, in the same `evict_before` call |
| Target memory not held by the receiver | Accepted; the counter row exists ahead of the memory, as a local verdict on an unknown id already does |
| Code target, receiver has no approved label for the repo | Accepted; counter keyed by the canonical name |
| Unscoped or unapproved target / activity run | Stays local; no event id; journals nothing |
| `auto_coactivation` verdict | Local only |
| Legacy code-target verdict | Local only (identity unrecoverable) |
| Backfill run twice / interrupted | Resumes from its cursor; stamped rows skipped; journals nothing twice; counters never touched |
| Activity capture fails mid-batch | Batch transaction rolls back; cursor unmoved; next call retries |
| Query text matches a secret rule | `query` omitted, `query_withheld: true`; not logged; event still shared |
| Query text holds an absolute path | Path token becomes `<path>` |
| `activity.enabled = false` on the source | No row, no event |
| `activity.summaries = false` on the source | Shared event with `summary: null` |
| `activity.enabled = false` on the receiver | Feed position + receipt; no `activity_log` row |
| Payload carries a key outside the allowlist, or a disallowed command | `rejected_invalid` |
| `tombstone`/`restore` for an event kind | `rejected_invalid` |
| Event-kind upsert with a digest and no payload | `payload_expired` receipt; no position; no contribution |
| Event past retention on this engine, offered again | `payload_expired`; no contribution |
| Event whose target memory was purged here, offered again | `payload_erased`; neither row nor counter restored |
| Two events with the same `at` | Ordered by acceptance in the feed and the activity cursor; neither skipped |
| Receiver imports its own device's event it no longer holds | Accepted as that device's event (the attribution is true); the unique `event_id` index still forbids a second row |
| `comemory rebuild` | Device id, event ids, `device`/`surface`/`actor` columns and payload redactions survive |

## Acceptance criteria

- **AC-1:** (V-1) A real scoped `comemory find` followed by a real
  `comemory feedback` with one memory and one code verdict, run with
  `COMEMORY_ACTOR` set, stamps an `ev-` event id on both `feedback_events` rows
  and journals one `feedback_event` per verdict. Each payload carries the
  source device id, the row's `at`, `surface: "cli"`, the actor, and
  `provenance: "manual"`. After transfer, the receiver's two rows carry the
  sender's device, `at`, surface and actor, and not the receiver's device id.
- **AC-2:** (V-1, V-2) The same run's `find` activity row is journalled as one
  `activity_event` with its `source`, `actor`, `at`, device, and the canonical
  repository in place of the local label. The receiver's imported
  `activity_log` row carries the sender's device, and `GET /api/v1/activity` on
  the receiver shows it with that `device`, while its own runs show
  `device: null`. An activity row recorded before capture existed (the upgrade
  state: no event id, no journal row) is journalled once by the sweep, and
  further sweeps neither re-journal it nor change its event id.
- **AC-3:** (V-2) The memory verdict's target is the memory id. The code
  verdict's target is the canonical repo, the parent symbol's repo-relative path
  and name, and that row's `blob_oid`. `origin_query_id` is
  `<sender device>:<the find query id>`. The receiver gains no `retrieval_log`
  row. Its code counter is keyed by its own approved label, or by the canonical
  name when it has none.
- **AC-4:** (V-3) Two separate real `comemory feedback` calls on the same query
  and targets, transferred to a receiver, set its memory and code counters to 2.
  Re-posting the same envelope twice more, and posting the receiver's own feed
  back to the sender (echo), change no counter anywhere. Each replay answers
  `duplicate` with the original sequence.
- **AC-5:** (V-3) With a `BEFORE INSERT` trigger on `feedback` that raises,
  importing a verdict fails and leaves no `feedback_events` row, feed position
  or receipt. After the trigger is dropped, the same operation applies exactly
  once. A `serve` process killed with SIGKILL while applying an envelope of at
  least 200 real verdicts leaves, for every target,
  `counter == number of imported event rows`, and a receipt and feed row for
  each imported event.
- **AC-6:** (V-3) A database holding real legacy verdicts (the state an upgrade
  from 0025 leaves: rows with no event id and no journal), where one verdict
  has already been evicted by `comemory gc`, backfills only the retained
  memory-target verdicts, stamping each with an `ev-` id. A second backfill
  pass journals nothing and changes no id. The sender's counters — which still
  include the evicted verdict — are byte-identical before and after both
  passes. A receiver's counter equals the number of retained events, not the
  sender's counter. The legacy code-target verdict is not journalled.
- **AC-7:** (V-4) Provenance survives transfer: `manual`, `implicit` and
  `auto_search_edit` arrive as stored. A real `auto_coactivation` verdict is
  never journalled. On the receiver, replicated `manual` and `implicit` verdicts
  mint no golden-harvest pair and mark no query successful for `mine`.
  `recall-status` reports the same `queries`, `feedback_events` and `pending`
  before and after the import, and the receiver's own pending query stays
  pending.
- **AC-8:** (V-5) Two engines exchange their complete `changes` feeds in both
  directions (A→B, B→A) five times over. The test drives each exchange itself,
  over real HTTP: `GET /api/v1/sync/replica/changes` on one engine, then every
  entry as an operation to `POST /api/v1/sync/replica/import` on the other —
  the push/pull client is #255. Between rounds the real
  `comemory sync --action auto` pass runs on both data directories without a
  login; it moves no replica events, and is there to prove the resident
  reconcile pass neither journals imports nor records shareable activity. After the first
  round each engine's feed head, local-origin positions, outbox and
  `activity_log` count stop changing: the exchange reaches a fixed point. No
  imported row is ever journalled with origin `local`. A legacy `sync.import`
  activity row, written by a real `POST /api/v1/sync/import`, is never
  journalled.
- **AC-9:** (V-5) A source with `activity.summaries = false` shares
  `summary: null`, and its query text appears in no `replica_payload` row and
  no `changes` response. A source with `activity.enabled = false` shares no activity
  event. A receiver with `activity.enabled = false` records the feed position
  and receipt but no `activity_log` row. A receiver with
  `activity.summaries = false` stores the row with a NULL summary.
- **AC-10:** (V-6) Unscoped `find` / verdicts and ones scoped to an unapproved
  repo journal nothing. A scoped `find` whose query contains a real secret-rule
  match is shared with `query_withheld: true`. The secret string appears in no
  `replica_payload` row, `changes` response, `events` frame, or captured
  `serve` / CLI stderr at `RUST_LOG=debug`. A query holding an absolute path is
  shared with `<path>` in its place. Shared summaries carry no key outside the
  allowlist. An envelope adding a non-allowlisted key is `rejected_invalid`.
  After a full transfer of an engine that ran real searches, `find` captures
  and verdicts, the sender's feed holds no entity kind beyond `memory`,
  `code_generation`, `document_revision`, `feedback_event` and
  `activity_event`, and the receiver's `retrieval_log`,
  `candidate_query_observations`, `candidate_observations`,
  `candidate_judgments` and `bandit_arms` are still empty. (The platform's
  remote UI is comemory.io #185's; the engine's evidence is what it serves.)
- **AC-11:** (V-7) After `comemory gc` evicts a transferred verdict and
  activity row past retention, the sender's `changes` lists both with
  `payload_state: "expired"`, a digest and no payload. The feed row, revision
  and receipt remain. Re-offering either event answers `payload_expired` with no
  counter change. An event-kind upsert with a digest and no payload answers
  `payload_expired`, and replaying it reads the same answer. Relaying the
  sender's whole `changes` page — expired entries included — to a fresh engine
  answers every operation (`payload_expired` for the expired ones, `accepted`
  for the rest), so a bootstrap walking that page advances to its end.
- **AC-12:** (V-7) Purging a memory through the real delete + `comemory gc`
  erases the journal copies of the verdicts on it (`payload_state: "erased"`).
  Replaying the original operations answers `payload_erased`, and no
  `feedback` or `feedback_events` row for that memory reappears.
- **AC-13:** (V-7) Two real activity events given the same `at` and imported in
  order X, Y come back X, Y from `changes` paged with `limit=1` and from
  `/api/v1/activity/events?after_id=`. Imported into a second engine in order
  Y, X, they come back Y, X.
- **AC-14:** `scripts/replication/coverage.json` maps `V-1`…`V-7` to case
  `events`. `bash scripts/test-replication-e2e.sh --case events` runs
  `replica_events`, `replica_events_2` and `replica_events_3` and exits 0.
  `bash scripts/check-replication-coverage.sh` exits 0.
- **AC-15:** `comemory rebuild` keeps the device id, every event id, the new
  columns, every payload redaction and every `activity_log` row. A verdict
  transferred after the rebuild is still answered `duplicate` by the rebuilt
  engine.
- **AC-16:** (V-2, V-3) A real scoped `find`, a real verdict citing its query
  id, then the `find`'s `retrieval_log` row evicted by a real `comemory gc`
  (its `at` aged past retention) before the transfer: the receiver stores the
  verdict with its provenance and `<device>:<query id>`, counts it once, and
  holds no `retrieval_log` row.

## Acceptance evidence

| AC | Real input | Expected observable | Boundary / failure | Runnable check |
| --- | --- | --- | --- | --- |
| AC-1 | Two spawned `serve` engines; real memories saved from repo README prose; a real git checkout indexed by `index-code`; real `find` and `feedback` through the CLI; approval rows written as `approve_docs` does | Event ids on rows; payload fields; receiver rows keep the sender's provenance | Receiver device ≠ sender device | `tests/replica_events.rs` |
| AC-2 | Same engines; real scoped `find` | Imported `activity_log` row + API `device` | Local rows `device: null` | `tests/replica_events_2.rs` |
| AC-3 | Chunked symbol from the indexed checkout | Target shape, `blob_oid`, namespaced query id, no `retrieval_log` row | Receiver with no approval: canonical key | `tests/replica_events.rs` |
| AC-4 | Two real feedback calls; envelope from real `changes` | Counters 2; replays and echo answer `duplicate` | Echo under the sender's own operation id | `tests/replica_events.rs` |
| AC-5 | Real trigger in the real DB; 500-verdict real envelope; SIGKILL | Rollback; invariant per target | Kill lands anywhere; invariant must hold | `tests/replica_events.rs` |
| AC-6 | Real verdicts, then the upgrade state (event ids and journal rows cleared, as bootstrap tests do); one real row evicted by real `gc` after its `at` is aged past retention | Retained verdicts journalled once; counters unchanged | Second pass journals 0; code legacy stays local | `tests/replica_events_3.rs` + `src/domains/sync/replica/tests/event_capture.rs` |
| AC-7 | Real `POST /api/v1/feedback` with `source: "implicit"` beside a CLI `manual` verdict; real `auto_search_edit` / `auto_coactivation` rewards through `record_implicit_used`, the writer `graph::coactivate` calls, over a real saved memory | Provenance preserved; harvest/mine/recall-status unchanged; only the search→edit reward journalled | Receiver's own manual verdict still harvested | `tests/replica_events_3.rs` + `src/domains/learning/tests/feedback_share.rs` + `src/store/tests/feedback.rs` |
| AC-8 | Two engines with real events on both; full-feed exchange both ways ×5; real `comemory sync --action auto` between rounds; a real `POST /api/v1/sync/import` | Fixed point after round 1 | `sync.import` row never journalled | `tests/replica_events_2.rs` |
| AC-9 | Real `config.toml` with the two `activity.*` keys on either side | As stated | — | `tests/replica_events_2.rs` |
| AC-10 | Scoped/unscoped real `find`; a query holding a string matching a `rules.toml` rule; a query naming an absolute path; serve stderr captured to a file | No secret anywhere; `<path>`; `query_withheld` | Crafted extra key refused | `tests/replica_events_2.rs` + `src/domains/sync/replica/tests/activity_payload.rs` + `src/utilities/tests/shared_text.rs` |
| AC-11 | Transferred events; the materialized rows' `at` aged past retention (retention must be ≥ 1 day), then real `gc` | `expired` state; `payload_expired` answers | Digest-no-payload upsert; the feed-`at` arm for a never-materialized import | `tests/replica_events_2.rs` + `src/store/tests/gc_learning.rs` |
| AC-12 | Real `delete`, the trash file's mtime and `deleted_at` aged past the (≥ 1 day) trash retention as `tests/cli__gc.rs` does, then real `gc` | `erased` state; `payload_erased`; no rows | An expired event on the same memory stays `expired`, not `erased` | `tests/replica_events_3.rs` + `src/store/tests/memory_purge.rs` |
| AC-13 | Two real captured activity payloads re-digested with one shared `at` (two devices' runs in one instant) | Acceptance order in both cursors | Reverse order on a second engine | `tests/replica_events_2.rs` |
| AC-14 | The manifest and runner | Exit 0 | Checker rejects a missing `V-*` key | `bash scripts/test-replication-e2e.sh --case events`; `bash scripts/check-replication-coverage.sh` |
| AC-15 | Stopped engine; real `comemory rebuild` | Values preserved (incl. activity rows); echo `duplicate` | Pre-0026 source columns copy as `NULL` | `tests/replica_events_3.rs` + `src/store/tests/rebuild_copy_learning_events.rs` |
| AC-16 | Real `find` + `feedback`; the query's row aged then evicted by real `gc` on the sender | Verdict on receiver, once, namespaced; no query row | Sender reports `known_query: true` at record time; the eviction happens after | `tests/replica_events_3.rs` |

No mocks: every check drives the real binary, real spawned `serve` engines,
real HTTP and real SQLite files. Backdating an `at` stands in only for the
passage of days, never for the behavior under test — the same pattern
`tests/cli__gc.rs` uses.

## Documentation impact

- This design, linked from `docs/README.md`.
- `docs/designs/2026-09-21-replica-v1-journal.md`: the two kinds, `expired`,
  and the event duplicate rule.
- `docs/guides/replication-e2e.md`: case `events`.
- `docs/guides/http-api.md`: the activity `device` field; the `expired` payload
  state.
- `docs/guides/cloud-sync.md`: exactly which history and counters are shared
  (the tables above, in user terms).
- `docs/guides/prune-and-gc.md`: gc expires shared event copies; purge erases
  them.
- `docs/architecture.md`: `replica_device`, the new columns.
- `src/store/README.md`, `src/domains/sync/replica/README.md`,
  `src/domains/learning/README.md`, `src/utilities/README.md`: new files.
- `AGENTS.md`: module-map lines for learning, sync and utilities.
- `docs/guides/runtime-orm.md`: the hand SQL in `store::replica_redaction`.
- `migrations/README.md` and the domain-first migration inventory: the new
  migration and files.

## Open Questions

None blocking. Decisions made from the issue, the epic and the code, with their
reasons stated above:

1. Scoped query text is shared (after path stripping and the secret scan). The
   "private query histories" exclusion is read as `retrieval_log`.
2. `auto_search_edit` is shared; `auto_coactivation` is not. The first is an
   observation only one machine can make; the second is re-derived by every
   indexing machine.
3. Legacy code verdicts are not backfilled; their identity is unrecoverable.
4. Receiver display settings govern the materialized activity row, not the
   journal.
5. Console read visibility for shared activity is comemory.io #185's decision.
   The engine exposes `device` and nothing else new.

## Review

Reviewed 2026-09-24 against the `spec-review` checklist, in two passes. The
first raised three blockers — the erased/expired readers were boolean, the
rebuild copy passes name their columns, and `activity_log` had no copy pass at
all — and eight should-fixes: `resolve_identity` visibility, the `duplicate`
receipt's sequence, where the event-kind shape check sits, purge losing event
ids before the delete, the `validate.rs` / `store` file ceilings, the
`bootstrap.state` merge rule, and a missing AC for the gc'd originating query.
The second pass raised the activity half of the gc expiry and how AC-8's
exchange is driven. All are resolved above. The one issue clause the engine
cannot evidence, the platform's remote UI, belongs to comemory.io #185.
**Status: Approved** for planning.

