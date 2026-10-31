# Bring your own vectors

**Goal:** add dense/semantic retrieval with a configured embedding provider or
explicit vectors. comemory ships no model, so you choose and run the embedder.

comemory works out of the box with lexical FTS5 search; no vector is required.
Without a configured provider, a save without a vector is **lexical-only** —
no `memory_vec` row is written, and `comemory search` still finds it through FTS5. Supplying a vector adds the
dense leg that gets fused with the lexical one.

## Configure automatic memory embeddings

Set `COMEMORY_EMBED_CMD` to a command that reads UTF-8 text on stdin and
writes `{"embedding":[...]}` on stdout. It must return 1024 finite values.
Unset or blank disables automatic embeddings. Ordinary save, search, find and
context calls use it; explicit vectors always take precedence. `serve
--embed-cmd` overrides the environment for that server. Restart the server or
MCP host after changing its environment: an export in a terminal does not
configure a GUI host already running. Configure the host's process environment
without placing credentials in shared instructions.

For example, point the command at your own executable that accepts text on
stdin and emits the JSON payload described above, then use normal commands:

```bash
export COMEMORY_EMBED_CMD=/path/to/text-to-vector-command
# Optional separate providers, with model identity labels:
export COMEMORY_DOCUMENT_EMBED_CMD=/path/to/document-vector-command
export COMEMORY_DOCUMENT_EMBED_HINT=my-document-model
export COMEMORY_CODE_EMBED_CMD=/path/to/code-vector-command
export COMEMORY_CODE_EMBED_HINT=my-code-model
comemory save "Use Postgres for analytics" --kind decision
comemory search "analytics database"
comemory run embedding --path .
comemory find "analytics" --domain document
```

Memory and document providers must emit 1024 values; the code provider must
emit 768. These paths are examples; supply your own compatible executables.

A failed automatic save embedding returns a warning while retaining the
markdown and lexical mirror; retry the same save after fixing the provider to
repair its vector. A failed query embedding returns an error. Doctor reports
whether the command is configured and whether a probe worked; the probe checks
width, not model compatibility. Each provider invocation has a 120-second
deadline; for long documents configure the provider's context and batch limits rather
than truncating content. Existing memories are not backfilled on startup,
import, restore, or rebuild. Use the explicit `POST /api/v1/doctor/reembed`
operation to backfill them.

Use the same model and preprocessing for saved bodies and queries: matching
widths alone do not establish compatibility. `COMEMORY_EMBED_HINT` is a label,
not enforcement. A model change requires explicit re-embedding.

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
branches run and their results are fused via RRF; without one, a configured provider supplies the memory query vector; otherwise only the lexical
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

Vector dimensions are **fixed** and are not env-configurable. Memory and code
dimensions are baked into `vec0` DDL at migration time:

- Memory vectors: **1024** dimensions (`memory_vec`).
- Document vectors: **1024** dimensions (`document_vectors`).
- Code vectors: **768** dimensions (`code_vec`).

Always embed with a model whose output dimension matches. A wrong-length vector
fails fast with `VecDimMismatch` (a dim guard checks the vector against
`schema_meta` at first insert) rather than corrupting the index. Models must emit the dimension required for their domain; changing it requires
coordinated schema and implementation changes. See
[architecture](../architecture.md).

## Recording which embedder

Set `COMEMORY_EMBED_HINT` to a free-form label of your 1024-dimensional memory
embedder, e.g.:

```bash
export COMEMORY_EMBED_HINT="my-1024-dimensional-memory-model"
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

`rebuild` rebuilds `comemory.db` from `memories/*.md` (snapshotting the old one
to `comemory.db.pre-rebuild.bak` first), so no saved memory is lost. Note that
memory **vectors are not restored** — `memory_vec` is deliberately left empty,
because re-embedding means re-running your embedder. Re-save with an explicit
vector or configured provider, or run the explicit re-embedding operation to repopulate them; lexical search works in the meantime.
Code and document vectors are copied from the previous database.

## See also

- [CLI reference](../cli-reference.md) — every flag for `save`, `search`,
  `search-code`, `context`, and `rebuild`.
- [Configuration](../configuration.md) — `COMEMORY_EMBED_HINT` and the
  non-configurable `1024` / `768` vector dimensions.
- [Getting started](../getting-started.md) — the save / search / index loop.
- [Architecture](../architecture.md) — the save flow, the vector tables, and
  where the dimension literals live.

## Explicit folder reconciliation

Run `comemory run embedding [--path DIR] [--repo LABEL] [--mode incremental|full]`
for supported documents and code. No flags selects exactly the current directory;
Git, a project manifest, and `comemory init` are optional. Relative paths resolve
against the invocation directory. An ancestor Git repository never widens it.
`--repo` changes metadata/filtering only. Canonical-root source identities isolate
folders with the same name. Source registration rejects overlapping roots.

TXT, Markdown, HTML/XHTML and CSV/TSV use the existing document extractors;
Rust, TypeScript, JavaScript, Python and Go use code extraction. Unsupported,
binary and oversized items are reported. PDF, office, OCR and audio extraction
are unsupported. Source files are never rewritten. Hidden-file, Git ignore,
`.comemoryignore`, managed-memory and data-directory boundaries apply.

Documents use a separate 1024-dimensional provider selected by
`COMEMORY_DOCUMENT_EMBED_CMD` and `COMEMORY_DOCUMENT_EMBED_HINT`. Stored vectors
belong to a hash of that command/hint and raw-chunk preprocessing version, tied
to the document revision and chunk ordinal. Provider changes make old vectors
ineligible; ordinary indexing retries missing compatible vectors. The additive
vector table cascades deletions from document chunks. Document retrieval fuses
lexical matches with provider-scoped cosine nearest neighbors and reports their
contribution as `score_parts.ann`, preserving source/chunk citations.

Memory and document queries share one inference only when command, hint and raw
text representation agree. Otherwise each selected domain embeds separately.
Explicit vectors invoke no providers and target only the compatible memory or
code leg; documents remain lexical in that request. `search` and `context` keep
their memory contracts; semantic documents are available through `find`.

`COMEMORY_CODE_EMBED_CMD` selects a separate 768-dimensional code provider. Its
command and `COMEMORY_CODE_EMBED_HINT` identify managed vector generations.
Code-only queries still require an explicit compatible vector for semantic
search. Never truncate or pad a vector to cross model spaces.

Incremental runs hash actual file bytes, preserve unchanged compatible vectors
and retry missing embeddings. Full runs re-extract selected code and refresh configured managed code and
document embeddings, retaining valid identical-content vectors if refresh fails. Inference happens outside SQLite write transactions.
Late publication verifies source content and database identity; stale results
are discarded for retry. A failed code scan rolls back its text transaction;
document indexing publishes per file and suppresses missing-file reconciliation
after an incomplete walk. Cancellation can retain already committed document
files or lexical code, with missing embeddings repaired on the next run.
Unavailable roots are errors, never an empty authoritative scan.

`comemory index` remains document-only and shares that lifecycle; `index-code`
remains a deprecated code-only compatibility entry with its existing flags and
read-only `--extract` mode. Git hooks and HEAD-triggered lazy indexing use the
shared code core and inherit their process environment. They do not detect every
unstaged edit; use an explicit folder run for working-tree reconciliation.
