# Keep the code index fresh

**Goal:** make `search-code` and `context` return results that reflect your
latest commits, without re-running `index-code` by hand every time.

comemory stores code symbols in a SQLite index that is built by
`comemory index-code`. After you commit, that index can drift behind your
working tree. The `COMEMORY_INDEXING_AUTO_REINDEX` setting controls how (and
whether) comemory refreshes the index for you.

## Pick a mode

Set `COMEMORY_INDEXING_AUTO_REINDEX` to one of three modes (default `lazy`):

| Mode   | What it does                                                        | When to use                                                       |
|--------|---------------------------------------------------------------------|-------------------------------------------------------------------|
| `lazy` | A search whose repo HEAD moved spawns a background `index-code`.     | Default. You want freshness with zero setup and no query latency. |
| `hook` | Git hooks run `index-code` on commit / merge / checkout.            | You want the refresh to happen at commit time, not at query time. |
| `off`  | Nothing automatic; you run `index-code` yourself.                    | Scripted or CI pipelines that index explicitly.                   |

```bash
export COMEMORY_INDEXING_AUTO_REINDEX=lazy   # or hook, or off
```

You can also set it once in `~/.comemory/config.toml`; the environment value
wins over the file.

## How lazy mode works

`lazy` is the default and is fully **wired** (see `src/cli/lazy_reindex.rs`).
When you run `search-code` or `context` inside a git repo:

- comemory cheaply checks whether the repo HEAD moved since the last index.
- If it moved, comemory spawns a **detached, non-blocking** `index-code` in
  the background and returns immediately.
- Your query searches the **current** index right away — it never waits on the
  reindex. The *next* query sees the refreshed index.

So the freshness cost is paid in the background: the first search after a
commit may still hit slightly stale rows, and the following search is current.

### Debounce

A burst of searches during a rapid sequence of HEAD changes (for example a
rebase) could otherwise fork a herd of `index-code` processes. The
`auto_reindex_threshold_ms` config knob (default `200` ms) debounces this: a
trigger younger than the threshold suppresses a fresh spawn. Set it in
`~/.comemory/config.toml`:

```toml
[indexing]
auto_reindex_threshold_ms = 200
```

Lazy reindex only runs from the checkout it originally indexed (once a repo has
a recorded working-tree root). The exception is a repo with no recorded root —
never indexed via `index-code`, e.g. one populated through `ingest-code` — whose
first lazy reindex runs from the current directory. It is best-effort
throughout: if a reindex cannot start, the search still succeeds against the
current index.

## Hook mode

In `hook` mode comemory does nothing at query time; instead git hooks run the
refresh. Install them once per repo:

```bash
comemory install-hooks
```

This installs `post-commit`, `post-merge`, and `post-checkout` hooks that
trigger `comemory index-code`, so the index refreshes whenever your HEAD moves
through git. Re-run with `--force` to overwrite existing hooks.

## Off / manual

In `off` mode nothing is automatic — refresh the index yourself:

```bash
comemory index-code --repo myrepo --path .
```

Run this after the commits you want reflected in `search-code` / `context`.

## Verify

After switching modes, confirm freshness:

1. Commit a change that adds or renames a symbol.
2. Run `comemory search-code "<that symbol>"`.
3. In `lazy` mode, run it a second time — the new symbol should now appear
   (the first query triggered the background reindex).

## See also

- [CLI reference](../cli-reference.md) — `index-code`, `search-code`,
  `context`, and `install-hooks` flags.
- [Configuration](../configuration.md) — `COMEMORY_INDEXING_AUTO_REINDEX` and
  the `indexing.auto_reindex_threshold_ms` debounce knob.
- [Getting started](../getting-started.md) — the save / search / index loop.
- [Architecture overview](../architecture.md) — the code-indexing flow.
