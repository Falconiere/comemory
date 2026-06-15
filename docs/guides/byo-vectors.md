# Bring your own vectors

**Goal:** add dense/semantic retrieval to comemory by supplying your own
embeddings — comemory ships no embedder, so you choose the model and feed it
the vectors.

comemory works out of the box with lexical FTS5 search; no vector is required.
A save without a vector is **lexical-only** — no `memory_vec` row is written,
and `comemory search` still finds it through FTS5. Supplying a vector adds the
dense leg that gets fused with the lexical one.

You pass vectors two ways:

- `--vector` — a comma-separated list of floats (CSV).
- `--vector-stdin` — a JSON `{"embedding":[..]}` payload read from stdin.

## Embed and save a memory

The repository ships a sample Ollama wrapper,
[`scripts/comemory-embed.sh`](../../scripts/comemory-embed.sh), that embeds the
body and pipes the result into `comemory save --vector-stdin` for you:

```bash
scripts/comemory-embed.sh save "Use Postgres for analytics" \
  --kind decision --repo myrepo
```

The wrapper is documentation, not enforcement — swap Ollama for OpenAI, Voyage,
llama.cpp, or anything that emits a float array. The underlying call it makes is
just `--vector-stdin`:

```bash
echo '{"embedding":[0.1,0.2,...]}' | comemory save "Use Postgres for analytics" \
  --kind decision --repo myrepo --vector-stdin
```

With `--vector-stdin` the body must be a positional argument (stdin is taken by
the embedding payload). To pass the vector inline instead, use the CSV form:

```bash
comemory save "Use Postgres for analytics" --kind decision --repo myrepo \
  --vector 0.1,0.2,0.3,...
```

## Embed and search

Queries accept the same two flags. With a vector, both the ANN and lexical
branches run and their results are fused via RRF; without one, only the lexical
path runs. Route a query through the same model with the wrapper:

```bash
scripts/comemory-embed.sh search "what database do we use"
```

Or supply the query vector directly:

```bash
# JSON payload on stdin
echo '{"embedding":[0.1,0.2,...]}' | comemory search "what database do we use" --vector-stdin

# CSV form
comemory search "advisory lock" --vector 0.1,0.2,0.3,...
```

The same `--vector` / `--vector-stdin` flags work for `comemory search-code` and
`comemory context`.

## Dimensions

Vector dimensions are **fixed** and baked into the `vec0` DDL at migration time —
they are not env-configurable:

- Memory vectors: **1024** dimensions (`memory_vec`).
- Code vectors: **768** dimensions (`code_vec`).

Always embed with a model whose output dimension matches. A wrong-length vector
fails fast with `VecDimMismatch` (a dim guard checks the vector against
`schema_meta` at first insert) rather than corrupting the index. To use a model
with a different output dimension, change the literal in the DDL — see
[architecture](../architecture.md).

## Recording which embedder

Set `COMEMORY_EMBED_HINT` to a free-form label of the embedder you used, e.g.:

```bash
export COMEMORY_EMBED_HINT="ollama:nomic-embed-text"
```

It is surfaced by `comemory doctor` so you can confirm which model produced the
stored vectors. It is purely a label — comemory never consumes it as a switch.

## If a save half-fails

Markdown is the source of truth. If the SQLite mirror write fails after the
markdown file is written, the markdown file is **kept** and the error points you
at the fix:

```bash
comemory rebuild
```

`rebuild` drops `comemory.db` and repopulates it from `memories/*.md`, so no
saved memory is lost.

## See also

- [CLI reference](../cli-reference.md) — every flag for `save`, `search`,
  `search-code`, `context`, and `rebuild`.
- [Configuration](../configuration.md) — `COMEMORY_EMBED_HINT` and the
  non-configurable `1024` / `768` vector dimensions.
- [Getting started](../getting-started.md) — the save / search / index loop.
- [Architecture](../architecture.md) — the save flow, the vector tables, and
  where the dimension literals live.
