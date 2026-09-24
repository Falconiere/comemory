# Replicate document revisions under a name that does not depend on where the file sits

**Date:** 2026-09-23   **Issue:** [#253](https://github.com/Falconiere/comemory/issues/253)   **Builds on:** [the replica-v1 journal](2026-09-21-replica-v1-journal.md) (#250), [memory mutation capture](2026-09-22-memory-mutation-capture.md) (#251) and [code generation replication](2026-09-22-code-generation-replication.md) (#252)

The third entity kind: what one machine extracted from a document, carried to a
machine that may not hold the file at all.

## The identity problem this solves

A document's local identity is a hash of this machine's absolute path —
`source_id = SHA-256(canonical_path)`, then `file_id`, then `document_id`. Two
machines that index the same `README.md` from two checkouts agree on nothing,
and the identity is itself a machine path, which must never cross the wire.

A **shared id** is the digest of the canonical repository and the document's
normalized repository-relative path:

```
shared_id = hex(SHA-256(canonical_repo ++ NUL ++ normalized_path))[..32]
```

Neither input is a machine path. The local id is untouched: `document_share`
maps one to the other, so a portable name exists *beside* the local one rather
than replacing it.

The path is relative to the **repository root**, not the source root — a source
may be registered at any depth, and two machines that chose different depths
must still agree. A path that escapes the repository, is absolute, or
case-folds onto a name another local document already claims is refused rather
than merged: two unrelated files sharing one name would overwrite each other on
every peer.

## The repository has to be answerable offline

The canonical repository is an *input to the identity*, so capture cannot
proceed without it — and capture happens inside `comemory index`, which has no
network and no credentials. The workspace allowlist, however, is only ever
visible inside a policy fetch.

`repository_approval` closes that gap: the label-to-canonical map that
`RepositoryPolicy::persist_resolved_labels` writes whenever a policy loads,
replaced wholesale so a revoked repository stops resolving, and read offline by
both the capture and `comemory sources`. Absence is a refusal, not an unknown —
before the first policy load nothing is approved, which is the right answer for
a machine that has not joined a workspace.

Four things can withhold a document, and `comemory sources` names which:

| Reason | Fix |
| --- | --- |
| `no repository label` | register the source with `--repo` |
| `no sync policy has been loaded` | `comemory auth login` |
| ``repository `<label>` is not approved`` | approve it in the console |
| ``repository `<label>` has no indexed root on this machine`` | `comemory index-code` the checkout |

A withheld document is indexed and searchable locally exactly as before.
Withheld means unshared, not unread.

## Capture rides the transactions that already exist

Indexing a document appends exactly one operation inside `write_indexed`'s
existing transaction; removing one appends exactly one tombstone inside the
reconcile transaction. Capture is here rather than at push time — where a code
generation's happens — because a deletion has to be recorded when it happens:
once the tombstone has run, the rows describing what was removed are gone, and
a later push would have nothing left to tell a peer.

Sharing the transaction is also what keeps the two from disagreeing. A document
that exists locally but owes no upload is the divergence the journal exists to
prevent.

**A deletion is only journalled from an authoritative walk.** `discover`
previously skipped an unreadable entry with a warning and returned a short
candidate list, and `reconcile_deletions` treated that list as complete — so a
permission-denied subdirectory tombstoned files that still existed. Locally that
self-heals on the next scan; replicated, it does not, because peers drop a
document that exists and the next complete walk will not re-tombstone a row
already marked deleted. `discover` now reports whether it read everything, and a
run that did not makes no deletions at all, reporting `walk_complete: false`.

The deletion pass runs *before* the per-file loop, so a rename journals the old
path's tombstone ahead of the new path's revision and no moment has both rows.

## Nothing secret, and nothing local, leaves

The curated secret scan runs over the whole revision — title, every passage,
every heading breadcrumb, every link target — before anything is journalled. One
match blocks the revision whole: it is shared entirely or not at all, and
`document_share.blocked_reason` records which rule, visible in
`comemory sources`.

The rule set lives in `utilities::secret_scan` rather than in either capability.
The scan must run before journalling, which happens inside `domains::documents`,
and that capability may not depend on `domains::sync` — the reverse edge already
exists so acceptance can decode the payload, and both directions would leave
neither capability readable alone.

## Two parallel bodies of knowledge

A pulled revision never writes a local row. `documents`, `document_chunks` and
`document_fts` belong to what this machine indexed; `remote_document`,
`remote_document_chunk`, `remote_document_link` and `remote_document_fts` belong
to what a peer sent. An import into a machine with a real checkout leaves every
local row byte-identical and writes no file.

Links are carried, not injected. A pulled revision's outbound links land in
`remote_document_link` and are read for provenance; they are deliberately not
written into `edges`, which is a local table an import may not touch.

The pulled cache holds **one row per document**, replaced wholesale — the last
complete accepted revision wins and the one before it is gone. There is no
`state` column, unlike `code_generation`: a code generation is staged locally
before upload and needs a lifecycle, while a pulled document has no local
staging and an upload still arriving never reaches acceptance at all (its parts
sit in `replica_staged_part`). What makes a revision appear whole is the
acceptance transaction — the row, its passages, its links and its searchable
rows commit together or not at all.

## One search leg over two indexes

`document_fts::search` unions the local and pulled halves in a single query,
with the limit above the union. That ordering is the point: one `k` per side,
trimmed afterwards, would make a local result's rank depend on how much a peer
happened to share.

```
local passages  ─┐
                 ├─▶ UNION ALL ─▶ ORDER BY score ─▶ LIMIT k
shared passages ─┘   (approved, minus anything local)
```

A document both sides hold is returned once, from the local row — the copy with
a file behind it. The precedence rule is a `NOT EXISTS` over `document_share`,
which is the only thing the two sides can be compared by: a local path is
relative to its source root, a shared one to the repository, so the paths are
not the same value and the mapping is what relates them. That asymmetry is
visible to `--path` globs, which see both shapes.

Approval is a subquery inside the shared half, not a filter applied afterwards.
Revoking a repository stops its revisions answering the moment the map is
replaced, and reapproving resumes them — with no re-indexing either way, because
the text never moved.

A hit says which side answered: `comemory search --only document --json` reports
`shared_from` and `revision` for a pulled passage, and both `null` for a local
one. A reader has to be able to tell there is no file to open.

## Rebuild and gc

All six tables are copied by `rebuild` — a pulled revision cannot be re-derived
from anything on this disk, the share mapping is the only record of what a local
document is called upstream, and the approval map is server state whose loss
would leave every document withheld until the next policy load. `source_roots`
keeps being reconstructed from `sources.toml`.

Listing a table in `COPIED_TABLES` satisfies the coverage test and copies
nothing; the pass has to be written. Both are now asserted against real rows
surviving a real rebuild.

`gc` touches none of it. It sweeps trash, soft-deleted memories, telemetry,
activity rows and abandoned staged parts, and none of those is a document.

## What this issue does not do

The client push and pull loop is #255. This issue owns the entity: its identity,
its payload, its capture, its acceptance and its readers. Nothing is enqueued on
`replica_outbox` — the feed position is the durable record, and a queue entry
with no consumer would be a claim this code cannot keep.
