# Capture every memory mutation, and recover an interrupted write

**Date:** 2026-09-22   **Issue:** [#251](https://github.com/Falconiere/comemory/issues/251)   **Builds on:** [the replica-v1 journal](2026-09-21-replica-v1-journal.md) (#250)

#250 gave the engine a journal. This is what makes every memory mutation
actually reach it, decides which writers deliberately do not, and closes the
window where a killed process left a memory on disk that the database had
never seen.

## The three questions

**Which writers owe an operation?** Every writer that changes a replicated
field. `save`, `update`, `restore` and `delete` already journalled. `prune
--apply` did not: it soft-deleted a low-value memory and told no peer, so the
next pull re-offered the memory and the prune undid itself every cycle. It now
journals like any other deletion.

**Which writers deliberately owe nothing?** `refresh_refs` re-pins a code
anchor, `reembed` recomputes a vector, and `rebuild` replays the whole mirror
from markdown. Each changes stored state; none changes the memory a peer
holds. They are *materialization*, and journalling them would mint revisions
that say nothing and hand peers positions for changes they cannot observe.
That boundary is now pinned by test: each case first proves the state it
claims not to journal actually changed, so the zero-count is evidence rather
than a tautology.

**What happens when a write is interrupted?** A memory write places its
markdown and then mirrors it into `comemory.db`. Between those two steps the
file exists and the database has never seen it: the memory is unfindable and
the upload it owes is recorded nowhere, and no later command could notice.

## The write intent

`memory_write_intent` holds one row per memory with a write in flight —
recorded before the markdown moves, cleared in the transaction that finishes
the write. It carries the `operation_id` the finished write will journal
under, so a write completed at recovery is one operation rather than two.

```
intent recorded ──▶ markdown moves ──▶ mirror + journal + clear   (one commit)
     │                     │                        │
   killed here         killed here             killed here
     ▼                     ▼                        ▼
  nothing on disk,   intent + markdown,      committed: nothing owed
  intent dropped     database blind
```

For `save` and `delete` the mirror and the journal commit together, so the
intent clears in that one transaction. For `update` and `restore` they are two
transactions, and the intent clears in the **journal's** — the last one those
writes owe. A row that landed without its operation is still unfinished.

`domains::memories::recover::reconcile` reads the outstanding intents at
startup and finishes each one: mirror and journal an interrupted write,
complete and journal an interrupted delete, or drop an intent whose markdown
move never landed. It journals only when the intent's operation has no feed
position yet (`replica_read::position_of`) — a mirror row's presence proves
nothing, because the edit and restore paths commit the two separately.

It takes `memory-save.lock`, the lock `memories::save` holds across markdown
staging and the mirror commit: that is the section it re-enters. It is invoked
from the three delivery seams — `cli::run`, `serve::serve`, `mcp::serve` —
because `store::connection::open` may not call into a domain (#177). The CLI
skips it when no database exists yet, and skips it entirely for `serve` and
`mcp`, which reconcile from inside their own startup where they know whether
the session is read-only. A read-only session writes nothing and leaves the
intent for the next writable open.

## Embeddings ride outside the digest

An `Operation` carries an optional `vector`, alongside the payload rather than
inside it. The digest is taken over `payload` alone, so re-embedding a memory
cannot mint a new revision and two engines holding the same text agree on its
identity whether or not either has a vector.

Both import wires reach one verdict through `domains::sync::vector_rule`. An
embedding is usable only from the model this engine queries with, at the
dimension its `vec0` table was built for; a vector it cannot compare against
is worse than none. Refusing one never refuses the memory: the text lands and
the id is recorded in `memory_needs_embedding` with a reason the operator can
act on — `model` (re-embed locally), `dims` (fix the peer), `absent` (the peer
sends no vectors yet). The replica manifest reports the count and `comemory
doctor` warns with the route that drains it.

The legacy path used to drop an unusable vector silently, which left an engine
that looked healthy and answered every semantic search a memory short.

## An import cannot overwrite what this machine owes

A memory edited here and not yet pushed exists in exactly one place: the
payload its pending outbox row holds. `validate::decide` refuses an operation
for such an entity as `rejected_stale`, per entity rather than as a global
import freeze. The consequence is intended — an engine that owes unpushed
changes refuses imports for those entities until it pushes, which is what
orders the two changes properly instead of losing one.

## Two defects the real-process tests found

`cli::run` reconciled before every subcommand, including `serve`, so a
`--read-only` session wrote through the recovery pass.

An accepted import copied every replicated field except `created`. Because
`created` is part of the payload it is part of the revision's identity, so the
engine acknowledged one digest and stored another: two engines that had each
written the same body independently could never agree on a manifest digest,
however many times they imported from one another. The import now applies the
arrived creation time.

## Rebuild policy

`memory_needs_embedding` is copied by a rebuild, narrowed to the memories the
markdown replay restored: markdown carries no vectors, so dropping the backlog
would report a clean engine, while a row naming a memory the engine no longer
holds would report one that does not exist. A foreign key cannot express that
— SQLite's `OR IGNORE` does not apply to foreign-key violations, so one orphan
would abort the whole rebuild.

`memory_write_intent` is not copied. A rebuild replays every memory from
markdown, which is the reconciliation an outstanding intent would ask for.

## Not in this change

The pull-side echo of `vector` on `GET /api/v1/sync/replica/changes`. No
acceptance criterion depends on it, a pulling peer re-embeds locally from the
text it already receives, and rendering it costs a revision lookup plus a
`vec0` blob read per entry on a page of up to 500. It belongs with the
pull-side work in a later child of #248.

## Evidence

`bash scripts/test-replication-e2e.sh --case memories` runs the two
real-process suites that claim M-1…M-8: two spawned `comemory serve` engines,
the real CLI binary, real HTTP, real markdown trees and real SQLite. The
crash instant itself cannot be hit deterministically from outside the process,
so the fixture produces its exact observable state with the real library API
and lets the real CLI recover from it; that the state is reachable is proven
by `domains::memories::journal`'s own tests.
