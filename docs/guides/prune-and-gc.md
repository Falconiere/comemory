# Prune, rebuild, and garbage-collect

**Goal:** keep `comemory.db` healthy — drop low-value memories and dead edges,
recover the index from markdown when it breaks, and purge stale telemetry —
without losing anything you meant to keep.

comemory accumulates three kinds of cruft over time: edges that point at
deleted rows, memories that have decayed below usefulness, and code symbols for
files that no longer exist. `comemory prune` finds them, `comemory consolidate`
reports the near-duplicates worth merging, `comemory rebuild` reconstructs the
database from the markdown source of truth, and `comemory gc` trims the
learning telemetry to its retention window.

## Preview prune candidates

Run `comemory prune` with no flags. It is a **dry run** — it scans and reports,
mutating nothing:

```bash
comemory prune
```

It detects three candidate classes:

- **Orphan edges** — `edges` rows whose source or target no longer resolves to a
  live row.
- **Low-value memories** — memories that have decayed below every retention
  floor at once (see [Tune what counts as low-value](#tune-what-counts-as-low-value)).
- **Stale code files** — `code_symbols` rows for files that are gone from disk.

`--limit` / `--offset` page the **report only**. They window the candidate
**list** printed to you; they do not change what gets deleted:

```bash
# second page of 20 candidates (display window only)
comemory prune --limit 20 --offset 20
```

Use `--json` for CI: `low_value_memories` and `stale_code_files` come back as
`Page` envelopes (`items`, `limit`, `offset`, `total`, `has_more`).

```bash
comemory prune --json
```

## Apply prune

Add `--apply` to execute the cleanup: soft-delete low-value memories (their
markdown moves to `memories/.trash/`), drop orphan edges, and remove stale code
symbols.

```bash
comemory prune --apply
```

**`--apply` always acts on the full candidate set.** Pagination is
**display-only** — `--limit` / `--offset` never scope the deletion. Running
`comemory prune --apply --limit 20` still soft-deletes every qualifying memory,
not just the 20 you saw in the report. Preview first, then apply.

Soft-deleted memories sit in `memories/.trash/` until `comemory gc` hard-deletes
them, so an over-eager prune is recoverable until you garbage-collect.

## Tune what counts as low-value

A memory is low-value only when it fails **all** of the following floors at once
(or is superseded by a live memory it hasn't been accessed since). Set these in
the environment or `~/.comemory/config.toml`:

| Variable | Meaning | Default |
|----------|---------|---------|
| `COMEMORY_PRUNE_MIN_ACTIVATION` | Activation floor (ACT-R scale); eligible when activation is below this. | `-2.0` |
| `COMEMORY_PRUNE_MIN_FEEDBACK` | Beta-feedback ceiling `[0.0, 1.0]`; eligible at or below this. | `0.25` |
| `COMEMORY_PRUNE_BELOW_QUALITY` | Quality `1..=5`; eligible at or below this value. | `2` |
| `COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS` | Grace window before a superseded-and-never-accessed memory becomes eligible. | `7` |

Tighten or loosen them, then re-run the dry run to see the new candidate set:

```bash
COMEMORY_PRUNE_BELOW_QUALITY=3 comemory prune
```

The grace window protects freshly-rebuilt databases, whose supersede edges all
carry rebuild-time timestamps. See the [CLI reference](../cli-reference.md) for
the full `prune` flag list and JSON report fields.

## Find near-duplicate memories

`prune` drops memories that decayed; `comemory consolidate` finds the opposite
problem — several memories saying nearly the same thing. It is **advisory and
read-only**: it never writes an edge, never soft-deletes, never touches the
markdown. Merging stays your call.

```bash
comemory consolidate
```

```
clusters : 1  (412 memories scanned, radius 8)

cluster 1  (3 members, max hamming 11)
  a1b2c3d4  quality 4  accessed 12  ← keeper
  9f8e7d6c  quality 3  accessed 2   hamming 7
  4c5d6e7f  quality 2  accessed 0   hamming 11

  merge with: comemory save "<merged body>" --supersedes 9f8e7d6c --supersedes 4c5d6e7f
```

Clusters come from the same 64-bit SimHash the save-time near-duplicate warning
uses, grouped **transitively**: A near B and B near C puts all three together
even when A and C sit further apart than the radius, which is why each cluster
reports its widest distance as `max hamming`. A wide `max hamming` on a big
cluster means the group chained together — read it before merging.

The **keeper** is the member comemory's own ranker already favours: highest
`quality`, then most retrieved, then most recently used, then highest PageRank,
with the id as a final tiebreak. It is a suggestion; the `merge with:` line
spells out the exact `--supersedes` flags to retire the rest.

| Flag | Effect |
|------|--------|
| `--radius <0..=64>` | Hamming radius. Defaults to `rank.near_dup_hamming` (8); `0` reports only identical fingerprints. |
| `--repo <name>` | Scan one repo instead of the whole corpus. |
| `--all` | Include clusters you already settled with supersede edges. |
| `--k`, `--offset` | Page the cluster list. |

Clusters where every member but one is already superseded from inside the
cluster are hidden by default — you handled them, so re-reporting them is
noise. `--all` brings them back, labelled `resolved`.

If the header notes memories with no fingerprint yet, the store predates the
simhash backfill: run `comemory rebuild` and re-run the report.

## Rebuild from markdown

Markdown under `~/.comemory/memories/` is the source of truth;
`comemory.db` is a derived mirror. When the mirror is damaged — or a `save`
half-failed and left the database behind its markdown — drop and repopulate it:

```bash
comemory rebuild
```

`rebuild` replays every memory row and its FTS5 entry from the markdown, along
with the re-materialized `relations`, `references`, and edges. What markdown
cannot supply — the code index, the document index, the mined code-graph edges,
and the learning-loop counters — is copied across from the previous database, and
`source_roots` is reconciled from `sources.toml`. Memory **vectors are not
repopulated**: the BYO-vector contract means re-embedding is the caller's job
(see the [BYO-vectors guide](byo-vectors.md)).

Before it swaps the new database into place, `rebuild` snapshots the live one to
`comemory.db.pre-rebuild.bak`, so a rebuild that turns out wrong is recoverable.
This is the recovery path whenever a command points you at it.

## Garbage-collect logs

`comemory gc` does two things: hard-deletes `memories/.trash/` entries — the
markdown file **and** the memory's rows in `comemory.db` — and purges learning
telemetry past its retention window.

```bash
comemory gc
```

Trashed markdown is reaped once older than `prune.trash_retention_days`
(default `30`, a file-only `[prune]` key in `config.toml`; the console edits
it through `PUT /api/v1/gc/policy`). Each reaped file takes its mirror rows
with it in one transaction: the `memories` row, its tags, FTS and vector
rows, every edge touching it, its pinned code references, and the `feedback`
counter and events keyed by its id. Nothing is left for `GET /api/v1/trash`
or `stats.trashed` to keep counting, and a live memory is never touched — the
row delete is guarded on `deleted_at`. The sweep also purges any soft-deleted
row past the window whose trash file is already gone, so a store swept by an
older `gc` (which unlinked files but left their rows behind) heals itself on
the next run. `--json` reports the row count as `purged_rows`, next to the
file count in `removed`.

The telemetry window is `COMEMORY_LEARNING_RETENTION_DAYS` (default `90`). It
applies to **raw** rows only — `retrieval_log` and `feedback_events`:

```bash
# tighten the telemetry window to a week
COMEMORY_LEARNING_RETENTION_DAYS=7 comemory gc
```

Aggregated `feedback` counters and mined `query_expansions` **never expire** —
`gc` keeps them no matter how old, so your learned ranking signal survives the
purge. The one exception is the counter row of a memory `gc` has hard-deleted:
it goes with the memory (a memory id is only ever reused by a byte-identical
re-save, which should not inherit the verdicts of a memory you deliberately
deleted). `retrieval_log` rows are never touched by a purge — a row is one
query, and `returned_ids` is a list, not a key.

## See also

- [CLI reference](../cli-reference.md) — full `prune`, `consolidate`, `rebuild`,
  and `gc` flags.
- [Configuration](../configuration.md) — the `COMEMORY_PRUNE_*` floors and the
  `COMEMORY_LEARNING_RETENTION_DAYS` window.
- [Getting started](../getting-started.md) — the save / index / search loop.
- [Architecture overview](../architecture.md) — markdown as source of truth and
  the SQLite mirror it backs.
