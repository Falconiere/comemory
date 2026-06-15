# Prune, rebuild, and garbage-collect

**Goal:** keep `comemory.db` healthy — drop low-value memories and dead edges,
recover the index from markdown when it breaks, and purge stale telemetry —
without losing anything you meant to keep.

comemory accumulates three kinds of cruft over time: edges that point at
deleted rows, memories that have decayed below usefulness, and code symbols for
files that no longer exist. `comemory prune` finds them, `comemory rebuild`
reconstructs the database from the markdown source of truth, and `comemory gc`
trims the learning telemetry to its retention window.

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

## Rebuild from markdown

Markdown under `~/.comemory/memories/` is the source of truth;
`comemory.db` is a derived mirror. When the mirror is damaged — or a `save`
half-failed and left the database behind its markdown — drop and repopulate it:

```bash
comemory rebuild
```

`rebuild` reconstructs the entire database from the markdown: every memory row,
its FTS5 and vector entries, and the re-materialized `relations`, `references`,
and edges. This is the recovery path whenever a command points you at it.

## Garbage-collect logs

`comemory gc` does two things: hard-deletes `memories/.trash/` entries, and
purges learning telemetry past its retention window.

```bash
comemory gc
```

The retention window is `COMEMORY_LEARNING_RETENTION_DAYS` (default `90`). It
applies to **raw** rows only — `retrieval_log` and `feedback_events`:

```bash
# tighten the telemetry window to a week
COMEMORY_LEARNING_RETENTION_DAYS=7 comemory gc
```

Aggregated `feedback` counters and mined `query_expansions` **never expire** —
`gc` keeps them no matter how old, so your learned ranking signal survives the
purge.

## See also

- [CLI reference](../cli-reference.md) — full `prune`, `rebuild`, and `gc` flags.
- [Configuration](../configuration.md) — the `COMEMORY_PRUNE_*` floors and the
  `COMEMORY_LEARNING_RETENTION_DAYS` window.
- [Getting started](../getting-started.md) — the save / index / search loop.
- [Architecture overview](../architecture.md) — markdown as source of truth and
  the SQLite mirror it backs.
