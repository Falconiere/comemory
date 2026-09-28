# `comemory erase`

Permanently erase one memory or one document. Unlike [`delete`](delete.md)
(reversible) and [`gc`](gc.md)'s trash purge (housekeeping — a peer can still
restore the memory), an erase removes the entity's text from every table,
every journal copy and the markdown tree, and keeps only what makes the
erasure stick: the feed rows, revisions, receipts and payload digests. Those
digests are the barrier — a replay of the original operation, or a peer's
restore carrying the erased bytes, is answered `payload_erased` and
materializes nothing. New content under the same key is a new revision.

A live memory, or a document this engine shares, gets a tombstone journalled
first, so the deletion reaches the workspace. A pulled document copy is erased
locally only. Pending upserts and restores for the entity are withdrawn
(`rejected`, `payload_erased`); a pending tombstone still goes. An unchanged
re-index of an erased local document records its share blocked `erased` and
journals nothing; the source file itself is never touched.

Rollback snapshots — `comemory.db.pre-v*.bak`, `comemory.db.pre-rebuild.bak`,
`comemory.db.pre-restore.bak`, `memories.pre-restore/` and `backups/*/` — still
hold pre-erase state. The report names the ones present
(`snapshots_with_prior_state`); deleting them is the operator's decision.

**Runnable tests:** `tests/cli__erase.rs`, `tests/replica_recovery_3.rs`

**HTTP:** `POST /api/v1/erase` — `{"memory": ID}` or `{"document": SHARED_ID}`
plus `"confirm": true`; mutating (`405` on a `--read-only` server),
confirm-gated (`400 confirmation_required`), `404 not_found` for an entity
never held — covered by
`tests/cli__erase.rs::post_erase_is_confirm_gated_answers_not_found_and_erases`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Meaning |
| --- | --- |
| `--memory <ID>` | Memory id to erase. Exactly one of `--memory` / `--document`. |
| `--document <SHARED_ID>` | Shared id of a document — one this engine shares, or a pulled copy. |
| `--confirm` | Required: the erase cannot be undone. |

## Scenarios

### erase-01 Erase a memory

- **Flags:** `--memory`, `--confirm`, `--json`
- **Setup:** a saved memory whose body carries a unique token
- **Command:** `comemory erase --memory <id> --confirm --json`
- **Expect:** `{kind: "memory", key, tombstoned: true, payloads_erased: 1,
  operations_withdrawn, staged_removed, replay_blanked, wal_truncated: true,
  snapshots_with_prior_state: []}`; `show <id>` exits 64; the token is in no
  file under the data directory.
- **Covered by:** `tests/cli__erase.rs::erase_memory_reports_what_it_removed_and_leaves_no_copy`,
  `tests/cli__erase.rs::erase_prints_a_text_summary_without_json`

### erase-02 The confirm gate and exactly one entity

- **Flags:** `--memory`, `--document`, `--confirm`
- **Command:** `comemory erase --memory <id>`; `comemory erase --confirm`;
  `comemory erase --memory <id> --document <shared-id> --confirm`
- **Expect:** exit 70 `confirmation required` with the memory untouched; clap
  usage exit 2 for none or both entities.
- **Covered by:** `tests/cli__erase.rs::erase_without_confirm_is_refused_and_erases_nothing`,
  `tests/cli__erase.rs::erase_names_exactly_one_entity`

### erase-03 An entity never held

- **Flags:** `--memory`, `--confirm`
- **Setup:** fresh data dir
- **Command:** `comemory erase --memory 0badc0de --confirm`
- **Expect:** exit 64 `not found`; no `comemory.db` is created.
- **Covered by:** `tests/cli__erase.rs::erase_of_a_memory_never_held_is_not_found_and_creates_no_database`

### erase-04 Erase on a hub leaves only the barrier (AC-7)

- **Flags:** `--memory`, `--confirm`
- **Setup:** a real hub; a client saves a memory carrying a unique token,
  finds it by other words, records a verdict and syncs; the hub edits it;
  a peer pulls it
- **Command:** `comemory erase --memory <id> --confirm` over the hub
- **Expect:** `wal_truncated: true`; the token is in no file of the hub's data
  directory outside the named snapshots; feed rows, revision and receipts are
  kept; the hub's pending upload is withdrawn; a replay of the original
  import and the peer's restore answer `payload_erased`; a peer pulling the
  whole feed again applies nothing; an ordinary delete + restore still works,
  and a memory `gc` purged stays restorable from a peer.
- **Covered by:** `tests/replica_recovery_3.rs::erase_leaves_only_the_barrier`

### erase-05 Erase a document on its origin and a peer (AC-8)

- **Flags:** `--document`, `--confirm`
- **Setup:** a shared `docs/guides` document indexed on one client and pulled
  by another
- **Command:** `comemory erase --document <shared-id> --confirm` on both
- **Expect:** the origin journals a tombstone, the peer does not; none of the
  chunk text is left in any table, journal copy or file on either side; a
  re-pull and a replay of the erased revision apply nothing; the source file
  is untouched; an unchanged re-index records the share blocked `erased`; an
  edit journals a new revision.
- **Covered by:** `tests/replica_recovery_3.rs::erased_documents_stay_erased`
