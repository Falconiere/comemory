# Replicate a code index as whole generations, never as source

**Date:** 2026-09-22   **Issue:** [#252](https://github.com/Falconiere/comemory/issues/252)   **Builds on:** [the replica-v1 journal](2026-09-21-replica-v1-journal.md) (#250) and [memory mutation capture](2026-09-22-memory-mutation-capture.md) (#251)

#250 made the journal entity-agnostic. #251 taught memories to use it. This is
the second entity kind: what one machine learned about a repository's shape,
carried to another machine that may not have the repository at all.

## What a generation is

A **generation** is one machine's complete answer to "what is in this
repository at this revision" — a manifest of `(path, blob_oid)`, the symbols
those files declare, and the import and co-change edges between them. It is
immutable, content-addressed, and replaces its predecessor whole.

```
indexed tree ──▶ manifest + symbols + edges ──▶ canonical bytes ──▶ id = digest
                                                                      │
                          one parent, one child ◀────────────────────┘
```

Whole is the load-bearing word. A reader caught between a half-written
projection and its activation would see a repository that never existed: some
files at the old head, some at the new, with import edges pointing at both.
So `code_generation` rows carry a state — `staged`, `active`, `superseded` —
and activation is one transaction that records the generation, replaces the
repo's projection, flips the previous generation to `superseded`, journals the
feed position and writes the receipt. There is no instant at which half a
generation is visible.

The id is the digest of the canonical payload, so two machines that indexed
the same tree mint the same id, and a payload whose contents were altered in
flight no longer owns its id and is refused before anything is written.

## Source never leaves

A generation carries no `snippet` and no source text of any kind — not by
filtering, but by construction: `remote_code_symbol` has no snippet column to
put one in, and the wire type has no field to carry one. The consequence is
deliberate and visible: a machine holding only a pulled generation answers
`comemory repos` and the code graph for that repository, and returns nothing
from `search-code`, because there is nothing to rank or display.

## Two parallel bodies of knowledge

A pulled generation never writes a local row. `code_symbols`, `code_vec`,
`code_fts`, `indexed_files` and `repo_marker` belong to whatever this machine
indexed itself; `remote_code_file`, `remote_code_symbol` and
`remote_code_edge` belong to what a peer sent. An import into a machine with a
real checkout leaves every local row byte-identical.

Two readers must nevertheless see both sides, and they say so explicitly
rather than each inventing a rule:

| Reader | What it does |
| --- | --- |
| `comemory repos` | Merges by label: a repo this machine indexed AND a peer shared is ONE row carrying both revisions (`last_head` and `shared_head`). A repo known only from a peer reports `status: "shared"` with no root and zero counters |
| the code graph | Unions `edges` with the active generations' `remote_code_edge`, inside the paginated store query so the window, the pre-window total and the ordering stay correct. An edge the local index already states is not repeated: one edge, at the local weight |
| `search-code` | Not unioned — see above |

Upload selection reads `code_generation.origin`: only a generation this
machine built is ever offered, which is how a replication loop is prevented
rather than detected.

## Concurrency, deletion, disconnection

**Two machines, one parent.** Each plans against the generation the repo is
at. The first accepted wins; the second is refused with a conflict, because
accepting it would union two heads into a tree neither machine has. It
replans against the new active generation.

**A tracked file is deleted.** Nothing announces it. The next generation
simply does not name the path, and activation replaces the whole projection,
so the path leaves the peer exactly once and a replay removes nothing further.
(A full re-walk is what forgets a deleted file's cursor locally — the
incremental walk only ever visits files that still exist.)

**A checkout is unmounted.** Nothing is deleted on the strength of a path this
machine cannot currently read. The push withholds the repo, its reason stays
readable as `missing_root`, its index is untouched, and reconnecting the
volume resumes pushing. A disconnect is not a delete.

**An authoritative empty generation.** A `tombstone` for this kind means the
repository still exists and holds nothing, so the projection is cleared. It
never means the sender lost its checkout.

## Large generations and unfinished ones

A repository-sized manifest exceeds the 5 MiB envelope, so it crosses through
the existing staged-part route: parts land in a table nothing else reads, and
activation assembles them and goes through the ordinary acceptance path. An
upload that never completes has published nothing — `changes` and `manifest`
are built from the feed.

Two changes make that safe over time. An upload that acceptance has *answered*
now discards its parts, accepted or refused alike: a receipt is keyed on the
bytes that arrived, so resending the same operation id with a part corrected
is a conflict, and the parts could never activate. And `gc` sweeps what is
left — staged parts and `staged` generations over a day old — while walking
past every active generation, its projection and every receipt, since sweeping
a receipt would turn a peer's retry into a second acceptance.

## Schema

`code_generation` (the lifecycle) plus the three `remote_code_*` projection
tables, added by `migrations/0024_code_generation.sql`. A rebuild copies
generations that are not `staged`, and narrows the three projection tables to
the generations that survived that filter — so a rebuilt store never holds a
projection whose generation is gone.

## Where it lives

| Module | What it owns |
| --- | --- |
| `store/schema_code_generation.rs` | The four declared tables |
| `store/code_generation.rs` | Row lifecycle: record, activate, supersede, read what a repo is at |
| `store/remote_code.rs` | The projection, written and read per generation |
| `store/remote_code_view.rs` | Repo-scoped reads over the ACTIVE generation, and the one definition of the shared half of the graph |
| `store/replica_sweep.rs` | The abandoned-stage sweep |
| `domains/code/generation.rs` | Build a generation from what the local index recorded |
| `domains/code/replica_payload.rs` | The wire payload, its canonical bytes and the id they earn |
| `domains/code/remote_view.rs` | The union local + shared for the inventory and the graph |
| `domains/sync/replica/code_accept.rs` | The one-transaction write half |

The payload lives under `domains/code`, not under `domains/sync`: the planner
needs it, and `domains::code -> domains::sync` is forbidden. Putting it there
also collapsed what would have been two independent derivations of the same
digest into one.
