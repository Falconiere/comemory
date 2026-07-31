# Unified Document Indexing and Search Design

**Date:** 2026-07-31

**Status:** Design approved in conversation; awaiting review of this written specification

## Summary

Comemory will treat memories, source code, and external text documents as three
domains in one local knowledge system. A single `index` command will classify
mixed files and route them to the existing code extractor or a new document
extractor. A single `search` command will retrieve all three domains, preserve
domain-specific ranking, and fuse the ranked lists without comparing
incompatible raw scores.

External documents remain the source of truth. Comemory stores extracted text,
provenance, search indexes, and relationship evidence in SQLite, but never
rewrites or moves a source document. The default freshness strategy is a live
watcher. A lazy strategy remains available per source, and automatically acts
as the fallback when the watcher is unhealthy.

Document extraction uses an exactly pinned Xberg library inside Comemory. Risky
parsing runs in killable worker processes spawned from the same installed
`comemory` executable. There is no separately installed helper, runtime,
server, or sidecar. OCR uses statically linked Tesseract and Leptonica and is
selected page by page only when native PDF text is missing or low quality.

Semantic enrichment remains optional and bring-your-own. Lexical search and
deterministic relationships always work. When an embedding command is
configured, memories, document chunks, and code symbols use one shared model
and vector space so Comemory can create cross-domain semantic relationships.

## Goals

- Index individual files or directory trees containing both code and documents.
- Support PDF, DOCX, TXT, Markdown, and the other text-document formats enabled
  by the embedded extraction library.
- Search memories, documents, and code through one public `search` command.
- Return typed results with file, page, heading, line, and passage provenance.
- Discover useful relationships automatically across standalone documents and
  software repositories.
- Keep external sources fresh through a default live watcher and an optional
  lazy reconciliation mode.
- Continue to provide useful deterministic and lexical behavior with no
  embedder, OCR download, or network access.
- Isolate malformed or adversarial documents from the main process and retain
  the last good indexed revision after an extraction failure.
- Preserve Comemory's local-first, single-installed-executable model.

## Non-goals

- Copying documents into Comemory or making extracted text authoritative.
- Editing, converting, or writing back to external documents.
- Shipping an in-process embedding model or an in-process LLM.
- Running Xberg, Tika, Docling, LibreOffice, or OCR as an external service.
- Indexing archives, executables, standalone images, logs, or structured data
  such as arbitrary JSON, YAML, and TOML by default.
- Accepting or persisting document passwords.
- Adding a document browser to the existing TUI or web code editor in this
  feature. Those surfaces use the unified router with an explicit domain
  filter while retaining their current presentation.
- Expanding the set of programming languages understood by the AST extractor.

## Public command model

### Index and source management

The public indexing surface becomes:

```text
comemory index [--refresh watch|lazy] [--repo NAME] [--strict] <PATH>...
comemory sources
comemory unindex <SOURCE_ID|PATH>
comemory watch start
comemory watch stop
comemory watch status
```

`index` accepts files, directories, or a mixture. It validates and canonicalizes
every input, rejects overlapping registrations, atomically records the source
settings, and then performs the initial index synchronously. Registering the
same canonical path again updates its settings and reconciles it. Source roots
may therefore be either a file or a directory. Per-file failures leave the
source registered so the watcher or next lazy reconciliation can retry them.

The default refresh mode is `watch`. `--refresh lazy` overrides it for the
registered source. The global default is `indexing.refresh = "watch"`; the
environment override is `COMEMORY_INDEXING_REFRESH=watch|lazy`. Re-running
`index` for an existing path is the way to change that source's mode.

Repository association is inferred from the nearest enclosing Git worktree.
For a directory containing nested repositories, each file uses its nearest Git
ancestor. `--repo NAME` is an explicit root-level override for both Git and
standalone sources.

An inferred label starts with the Git worktree basename. If a different
worktree already owns that label, Comemory appends `~` plus the first eight hex
digits of a hash of its canonical Git common directory and reports the resolved
label. The chosen label and nested-worktree mapping are persisted in the source
registry, so later path or repository discovery does not silently rename the
identity.

Only one canonical linked worktree for a Git common directory may be registered
at a time. Registering a second linked worktree is a usage error naming the
existing source, because simultaneous branches would otherwise alias the same
repository/path graph identity.

Non-Git code receives a persisted synthetic label made from the source basename
plus `~<source-id-prefix>` when `--repo` is absent. Non-Git documents remain
unassociated unless an explicit label is supplied. The web editor resolves any
code file through its source-file record and absolute path, not by guessing a
root from the repository label.

One `--repo NAME` applies to every path in that invocation. It may associate
external documents with the one code repository in the set. If code-bearing
inputs span multiple Git worktrees or multiple non-Git code roots, the command
rejects the shared override and asks for separate `index` invocations. Reusing
an explicit label for code from a different identity root is also a usage
error; document-only sources may reuse it for association.

Git-associated paths are normalized relative to the Git worktree root so
memory file/symbol references remain stable when a narrower subtree is indexed.
Paths under a non-Git source are relative to its registered directory; a
single-file source uses its filename. During migration, an existing explicit
repository label and its recorded index root take precedence over fresh Git
inference, preserving the namespace used by stored references and feedback.

`sources` reports the stable source ID, canonical path, source type, refresh
mode, repository association, last reconciliation, indexed/error/stale counts,
and current watcher state. `unindex` unregisters the source and removes only
derived database content no longer owned by a registered source. It never
deletes or changes the external files.

The following public commands are removed immediately, without aliases or a
deprecation period:

- `index-code`
- `ingest-code`
- `search-code`
- `install-hooks`

Invoking a removed command produces a usage error that names the replacement.
The hook refresh mode and `COMEMORY_INDEXING_AUTO_REINDEX` are removed; live
watching and lazy reconciliation are the only freshness strategies.

Replacement-aware errors are implemented as hidden, parse-only tombstones.
They perform no work and do not appear in help, generated CLI documentation, or
shell completions. Their guidance is `index-code` to
`index --repo <name> <path>`, `search-code` to `search --only code`,
`ingest-code` to the configured batch embedder plus `index`, and
`install-hooks` to `index --refresh watch` plus `watch start`. `doctor` also
detects Git hooks carrying Comemory's managed hook marker and reports the exact
files and removal instructions. It never deletes a hook automatically because
users may have added unrelated commands.

Removed flags receive the same actionable parse error without being accepted as
aliases: `serve --root` points to `index --repo <name> <path>`, and `serve
--embed-cmd` / `tui --embed-cmd` point to `[semantic] embed_cmd` or
`COMEMORY_EMBED_CMD`.

Code-only operations such as `ast` and `graph` remain because they perform
specialized operations rather than parallel indexing or search.

### Unified search

The public retrieval surface becomes:

```text
comemory search <QUERY>
comemory search <QUERY> --only memory,document,code
comemory search <QUERY> --explain
comemory context <KEY>
```

`search` includes all three domains by default. `--only` accepts one or more
comma-separated domains. A domain-specific filter selects that domain
implicitly: `--kind` selects memories, `--lang` selects code, and repeatable
`--mime` selects documents by exact normalized media type. `--repo` applies
wherever repository metadata exists. Repeatable `--path` uses Git-style globs
against the displayed normalized identity path—repository-relative for Git
content and source-relative otherwise—and applies to documents and code.
Repeated `--mime` and `--path` values are ORed within their field.
Contradictory filters are usage errors instead of silently returning an empty
set.

The existing `--since`, `--until`, and `--as-of` meanings are retained as
memory-created-time semantics and therefore imply `--only memory`. Combining
them with any explicit scope containing document or code is a usage error.
External-file modified time is intentionally not overloaded as memory creation
time.

`context` remains a headline-oriented command but consumes the same unified
retrieval router and graph bundle. The existing TUI memory/code tabs and web
code editor call that router with explicit domain filters; a UI redesign is not
required for this change.

Results use stable, namespaced public identifiers:

- `memory:<memory-id>`
- `document:<document-id>`
- `code:<code-entity-id>`
- `source:<source-id>`
- `file:<source-file-id>`
- `ghost:<ghost-id>` for an unresolved explicit target

Document, code, source, and file entity IDs are opaque 128-bit lowercase
hexadecimal values. Ghost IDs are the first 128 bits of SHA-256 over the source
entity ID, target kind, and normalized unresolved reference. Repository, path,
symbol, kind, signature, and source span remain structured result attributes,
never delimiter-parsed identity.

TTY output labels the domain and prints the most useful citation. JSON results
carry `domain`, `id`, `title`, `snippet`, `score`, `score_parts`, `stale`, and a
typed citation object. Document citations contain source ID, path, page when
available, slide or sheet/cell range when applicable, heading path, extraction
method, and character offsets. Code citations contain repository, path,
language, symbol, and line span. Memory citations contain the memory ID and
managed Markdown path.

The existing pagination envelope remains. A document may match through several
chunks but occupies one result slot; its best passage is the primary citation
and up to two additional supporting passages are included in JSON output when
present.

Feedback becomes domain-neutral:

```text
comemory feedback <QUERY_ID> --used <TYPED_ID>...
comemory feedback <QUERY_ID> --irrelevant <TYPED_ID>...
```

The code-specific feedback flags are removed. Evaluation golden sets use the
same typed identifiers.

`search --json` and `context --json` emit `schema_version: 2`. Context paginates
over the same fused typed primary hits as search, then attaches an ordered
`related` array of typed graph neighbors to each hit. Related nodes do not
consume primary pagination slots. The version-2 envelope includes a typed
`freshness` object with watcher health, whether lexical reconciliation
completed, pending enrichment counts by producer, and active/pending generation
IDs; TTY output shows a compact warning only when that state is degraded or
pending. The web editor's `/api/search` response also
moves to schema version 2: it retains human-readable repository, path, and top
symbol fields, adds opaque `file_id`, and never parses those attributes from an
ID. TUI labels continue to show repo/path/symbol data while actions and feedback
carry opaque typed IDs.

The editor's file API moves in the same breaking version. `GET /api/file` and
`PUT /api/file` accept only an opaque `file:<source-file-id>`, look up its file
row, resolve an owned file through the current source registry, and reapply
canonical containment checks. GET
returns `schema_version: 2`, structured repository/path metadata, the current
content hash as an edit token, and whether the file is editable for a reachable
live code file. A synthetic or unreachable code file returns a typed
`source_unavailable` response carrying its safe metadata but no content. Both
endpoints remain code-only; PUT requires that edit token to match and rejects
document, stale, unreachable, synthetic legacy, and read-only-session files
with a typed error.
The per-session `serve --root` escape hatch is removed; registering or adopting
a root uses `index --repo <name> <path>` so identity and containment do not vary
by server process.

`graph` remains a code-only export and moves to schema version 2 with opaque
`file:` and `code:` node IDs plus structured repository/path/symbol attributes.
Its JSON envelope and `edges --json` carry the same freshness metadata. `graph`
does not become the unified document graph. Unified relationship evidence,
source-membership nodes, and ghost targets are exposed by `edges --json`,
`search --explain`, and context bundles.

Retrieval logs use one `search` source plus a stored domain-scope bitset rather
than a separate `search-code` source. Existing memory and code query rows migrate
to memory-only and code-only scopes. Typed feedback resolves to the appropriate
memory, document, or code aggregate internally. Learning jobs retain the query
scope so they never train a domain from a query that excluded it.

Golden cases carry `domains` and typed `relevant` IDs. Missing `domains` and
legacy bare eight-hex IDs are read as memory-only for migration; newly written
cases always use the explicit form. `eval` runs each case through its declared
scope, `tune` and `bandit` change only knobs used by those scopes, and mined
query expansions are keyed by domain instead of leaking code-only vocabulary
into a memory-only route.

## Architecture and boundaries

The feature is divided into narrow components with owned interfaces:

1. **Source registry** owns durable source configuration and mirrors it into
   SQLite for queries and diagnostics.
2. **Classifier** decides whether a discovered file is ignored, code, a
   document, or unsupported. It does not parse content.
3. **Code adapter** invokes the existing AST extraction and code-index flow.
4. **Document adapter** owns Xberg integration and translates library output
   into Comemory-owned extraction types.
5. **Extraction supervisor** runs same-executable workers with time and resource
   limits. No Xberg type crosses this boundary.
6. **Index writer** atomically replaces one file's derived rows in SQLite.
7. **Semantic projection** owns the optional shared embedder protocol and vector
   table.
8. **Relationship engine** converts explicit, lexical, and semantic evidence
   into logical graph edges.
9. **Unified retrieval router** asks each domain for an ordered typed candidate
   list, fuses those lists, and returns common results.
10. **Refresh coordinator** implements watcher events, periodic reconciliation,
    lazy fallback, and work deduplication.

The top-level data flow is:

```text
registered source
  -> discover and classify
  -> code adapter | document extraction worker
  -> per-file atomic index replacement
  -> lexical result is searchable
  -> optional semantic projection
  -> relationship evidence refresh
```

Extraction and semantic failures do not roll back successfully committed
lexical content. Derived enrichment is retried independently.

The code adapter preserves existing import extraction, git co-change mining,
PageRank materialization, working-set metadata, and search-to-edit
reinforcement. It changes their invocation seam from `index-code` to the code
branch of `index`; the underlying code-specific intelligence is not flattened
into generic document logic.

## Durable source registry

Registered sources are stored in `~/.comemory/sources.toml`, under the selected
`--data-dir`. This file is authoritative because `comemory rebuild` must be
able to rediscover external sources after replacing the database. Writes use a
temporary file, `fsync`, and atomic rename, following the existing memory-save
durability pattern.

The top level contains a format version, a monotonically increasing registry
generation, and `[watcher] autostart = true|false`. That durable desired state
is separate from the source entries and survives a database rebuild; ephemeral
watcher liveness never lives in the registry.

Each entry contains:

- a generated stable 128-bit source ID encoded as lowercase hexadecimal;
- canonical absolute path;
- whether the source is a file or directory;
- `watch` or `lazy` refresh mode;
- optional explicit repository label;
- persisted discovered Git worktree-to-label mappings for mixed roots;
- an optional canonical identity base used to preserve a migrated path
  namespace independently of the watched boundary; and
- creation and settings-update timestamps.

Ephemeral status does not live in TOML. SQLite mirrors the registry and stores
reconciliation cursors, fingerprints, errors, and counts. Startup reconciles
the mirror from TOML, so a partially completed database migration is
recoverable.

Canonical registered paths may not overlap. This prevents the same file from
being indexed twice with conflicting refresh or repository settings. Symlinks
are not used to evade this rule.

## Classification and discovery

Discovery applies rules in this order:

1. Reject paths outside the canonical source boundary.
2. Apply `.gitignore`, hidden-file defaults, and `.comemoryignore`.
3. Exclude Comemory's managed memory directory from external document indexing.
4. Classify extensions handled by the existing AST extractor as code.
5. Classify supported text-document extensions and MIME signatures as
   documents.
6. Record the remaining file as ignored or unsupported for diagnostics.

Markdown outside the managed memory directory is a document. Plain text and
supported rich-document formats are documents. Code classification takes
precedence for known source extensions. MIME sniffing may confirm or reject an
extension, but an executable or generic archive is never parsed merely because
it has a document suffix. ZIP-based Office/OpenDocument and EPUB files must
pass their package-structure validation before extraction.

The initial document allowlist covers PDF; DOCX, PPTX, XLS/XLSX;
ODT/ODS/ODP; TXT, Markdown, reStructuredText, AsciiDoc, Org, and RTF; HTML,
XHTML, XML, LaTeX, Typst, and Djot; CSV/TSV; EPUB; and EML/MSG. A supported MIME
signature may add a file whose extension is absent, but it cannot override an
explicit executable, generic archive, standalone-image, or known-code
classification.

Directory symlinks are not followed. A symlinked file is accepted only when its
resolved target remains within the registered source. Hidden files are ignored
unless explicitly re-included with a negated `.comemoryignore` rule. Archives,
standalone images, logs, and arbitrary JSON/YAML/TOML are excluded by default.

## Document extraction

### Dependency and process model

Comemory pins Xberg exactly rather than using a semver range. The initial pin is
`xberg = "=1.0.6"`; upgrades require fixture and extraction-contract review.
Default features are disabled. The initial feature set is `tokio-runtime`,
`pdf`, `excel`, `office`, `html`, `xml`, `email`, `chunking`, `quality`,
`heuristics`, `ocr`, and `bundle-tessdata-eng`; Xberg's core handles the
remaining supported plain-text formats. Xberg server, archive,
code-intelligence, embedding, LLM, URL-ingestion, and model-heavy layout or
rotation features remain disabled.

Release artifacts have no runtime extraction dependency. Source builds require
the standard C++ and CMake toolchain needed to compile the statically linked
OCR libraries, but never require a Java or Python runtime, office suite, or
separately managed parser executable.

The published `xberg-tesseract 1.0.6` build script fetches mutable upstream
artifacts without verifying checksums, so Comemory does not consume it
unchanged. A repository-pinned `[patch.crates-io]` copy keeps the public API but
pins the exact Tesseract and Leptonica archives and verifies SHA-256 before
unpacking. The English fast trained-data file is vendored with its license and
checksum. The patched build accepts local archive paths for offline packaging;
when it fetches missing source archives, every byte is checksum-verified. This
is a build-time supply-chain patch, not a second runtime executable or service.

The main process never parses a rich document directly. It spawns a bounded
pool of hidden extraction workers from its own executable and exchanges
versioned JSON over standard input/output. A worker receives one file and an
immutable limits/configuration snapshot, then returns a Comemory-owned
`ExtractedDocument`. A crashed, timed-out, or over-budget worker is killed and
replaced. This is process isolation, but not a sidecar: installation and
runtime contain only the `comemory` executable.

Default limits are configurable and begin at:

- 100 MiB source-file size;
- 5,000 pages;
- 120 seconds per file;
- 1 GiB resident memory per worker;
- 256 MiB of normalized extraction output and 100,000 emitted chunks; and
- `max(1, min(4, available_parallelism))` workers.

The parent hashes the file before extraction and checks its fingerprint again
before committing. If the file changed during extraction, the result is
discarded and the path is requeued.

### Native text, OCR, and quality

PDF extraction is page-adaptive:

1. Extract native text and structural metadata.
2. Score each page for absence, fabricated text, and low usable-text quality.
3. OCR only the pages that fail those checks.
4. Merge native and OCR pages while retaining page provenance.

Documents record `native`, `ocr`, or `mixed` extraction method, plus confidence
and warnings. Search ranking applies the bounded confidence demotion defined
below but never hides a passage solely because OCR was used.

Tesseract and Leptonica are statically linked into `comemory`. English fast
trained data ships with the binary. Additional versioned language packs live
under `<data-dir>/models/ocr/`, which defaults to
`~/.comemory/models/ocr/`. Language selection checks explicit configuration,
document metadata, the system locale, and bundled English in that order.
Downloads happen only when OCR is needed, use pinned checksums and sources,
and default to enabled; `ocr.download_models = false` disables them. An empty
`ocr.languages` list selects the automatic order above. If an offline run lacks
a required pack, the file status becomes `needs_ocr_model`; native searchable
text, if any, remains available.

Password-protected content is marked `locked`. Comemory neither prompts for nor
stores passwords. Corrupt or unsupported files receive a typed status and
diagnostic rather than terminating the index operation.

### Normalization and chunking

The adapter emits normalized Markdown-like text while preserving headings,
tables, list structure, pages, and source offsets. A title is selected from
document metadata, the first heading, or the file stem in that order.

Xberg structural chunks are accepted first. Oversized chunks are split at
paragraph or sentence boundaries with a default ceiling of 2,000 Unicode
characters and 200 characters of overlap. Tables are kept intact when under
the ceiling; oversized tables split only at row boundaries. Every chunk stores
its heading path, page range, normalized character range, source offsets when
available, page/slide/sheet location where applicable, extraction method,
confidence, and a 64-bit SimHash. Public page and slide numbers are one-based;
normalized character offsets count Unicode characters, while source byte
offsets remain explicitly labeled as bytes.

Chunk identifiers are revision-local implementation details. Public graph and
search identities target the stable document, with chunks retained as evidence
and citations.

## Data model

Existing memory and code domain tables remain the authoritative lexical and
behavioral stores for those domains. The schema adds:

### `source_roots`

SQLite mirror of `sources.toml`, including source ID, canonical path, source
type, refresh mode, repository override, discovered repository mappings,
identity base, reconciliation status, registry generation, and timestamps. A
single schema-meta value records the completely mirrored registry generation;
rows from another generation are never eligible for watcher commits.

### `source_files`

One row per discovered candidate file, including unsupported candidates but not
paths excluded by ignore rules. It stores a generated 128-bit file ID, source
ID (nullable only for a synthetic legacy row), normalized relative path,
classification, MIME type, size, modification time, platform file identity
when available, SHA-256 content hash, current status, last good hash, last
indexed time, last checked time, resolved repository label, normalized identity
path, and typed error details. New documents and code symbols reference this row
so one file transaction can replace all owned derived content. Only migrated
legacy code whose root is unrecoverable uses a synthetic source-file row with a
stable generated file ID, nullable source-root ownership, preserved
repository/path identity, no canonical writable path, and status
`stale_unresolved`. It remains searchable and graph-addressable but cannot be
refreshed or edited. A later `index --repo` may adopt that row and preserve its
file ID only when repository/path and content/symbol evidence identify exactly
one real file; ambiguity leaves the synthetic row untouched and reports a
diagnostic. Thus every searchable code entity has a file ID even when migration
cannot recover its source.

The fast freshness check compares size and modification time. A suspected
change is confirmed with SHA-256 before extraction. File status is one of
`pending`, `indexed`, `stale`, `stale_unresolved`, `error`, `locked`,
`needs_ocr_model`, `unsupported`, or `deleted`.

### `documents`

One logical document per external document file, with a stable generated
128-bit ID, source-file ownership, title, author and format metadata, page count,
extraction method, aggregate confidence, language hints, and current revision
hash. The ID survives edits and confirmed renames in the live database.

### Code entity identity and semantic units

Each parent code symbol receives a generated 128-bit entity ID. A confirmed
file rename preserves the source-file ID and therefore makes the prior symbols
eligible for reuse. Reindexing matches symbols within
`(source_file_id, language, kind, qualified_name)` groups:

1. A one-old/one-new group reuses the ID.
2. Larger groups first pair unique normalized-signature hashes.
3. Remaining symbols pair unique exact body hashes.
4. Remaining mutual-nearest SimHash pairs reuse IDs only within the configured
   Hamming radius; source-span proximity breaks a non-semantic tie.
5. Any ambiguous or unmatched symbol receives a new ID, and the unmatched old
   ID becomes a tombstone rather than transferring feedback.

A qualified-name change intentionally creates a new identity. Inserting an
earlier same-name symbol cannot shift identities by ordinal. When a file rename
is inferred through its unique unchanged content hash, its source-file and code
entity IDs follow the same rules.

The semantic unit is the complete parent symbol when it fits the existing AST
chunk ceiling. For an oversized symbol, only its existing leaf AST chunks are
embedded. Every leaf semantic item maps back to the same parent code entity,
and retrieval coalesces them before domain ranking.

### `document_chunks` and `document_fts`

`document_chunks` stores ordered normalized passages and their complete
provenance. `document_fts` indexes title, headings, passage text, and path using
the existing identifier-aware tokenizer where appropriate.

### `semantic_items` and `semantic_vec`

`semantic_items` maps one embedding unit to its stable entity and current text
hash. Units are memory bodies, document chunks, and the conditional code units
defined above.
`semantic_vec` is one shared `sqlite-vec` table. Its dimension is created from
the first accepted semantic model and guarded by schema metadata.

The legacy `memory_vec` and `code_vec` tables are removed after migration. Their
vectors are not copied because their fixed dimensions and model identities are
incompatible and cannot form a trustworthy shared space.

### `enrichment_jobs` and generations

Embeddings and generated relationships use a durable outbox. The same lexical
transaction that changes searchable text:

- updates the semantic item's current text hash and marks it `pending` when a
  provider is configured or `unconfigured` otherwise;
- deletes the prior vector, making it ineligible immediately;
- removes revision-bound generated evidence involving the changed entity; and
- upserts unique jobs keyed by producer, entity ID, source revision, and model
  generation.

Thus a crash cannot occur between committing text and recording its enrichment
work, and an old vector can never rank new text. Jobs store state, attempt
count, lease owner/expiry, next-attempt time, and the last typed error. Expired
`running` leases return to `pending`. Retry starts at one second, doubles with
plus-or-minus 20 percent jitter, caps at five minutes, and continues at that
rate until the source/configuration changes or the job succeeds. `doctor`
surfaces persistently failing jobs.

The durable producers are semantic embedding, deterministic references,
lexical relationships, repository post-processing, and semantic-graph rebuild.
`save` and `index` attempt a bounded synchronous entity-local enrichment drain
for the entities they changed: at most one semantic batch plus inline-safe jobs
for that batch, bounded by `semantic.batch_size` and `semantic.timeout_secs`.
The watcher drains the remaining queue.
Retrieval never inherits an unbounded backlog: after required lexical
reconciliation, the shared preflight leases at most
`indexing.query_enrichment_max_jobs` jobs and stops at
`indexing.query_enrichment_budget_ms`. Only deterministic-reference and lexical
relationship jobs explicitly marked `inline_safe` are eligible. Background
embedding jobs, repository-wide post-processing, and full-corpus semantic-graph
rebuilds never run inline with a query. The one query-embedding call needed for
semantic retrieval is not an outbox drain and remains bounded by
`semantic.timeout_secs`. An `inline_safe` producer must operate on a bounded
candidate set and honor the shared deadline through SQLite's progress handler;
cancellation returns its lease to `pending` without exposing partial evidence.

The default query budget is 25 milliseconds and 32 inline-safe jobs. A deadline
or job-count hit leaves work durably queued and adds `enrichment_pending` counts
and generation details to the result envelope; it does not make lexical search
fail. Full-corpus semantic passes run only in the watcher/coordinator or as part
of `rebuild`. If autostart is deliberately disabled, such work may remain
pending until `watch start` or `rebuild`, and `doctor`/retrieval diagnostics say
so explicitly.

An embedding job is due only while an embedding provider is configured. With
no provider, new semantic items use `unconfigured` rather than a failing
`pending` state and indexing does not create a retry storm. Enabling or changing
the provider advances its configuration generation and transactionally queues
all live units that lack a current vector for that generation. Removing the
provider parks outstanding embedding jobs as `provider_unconfigured`; `doctor`
reports semantic enrichment as disabled rather than failed. Deterministic,
lexical, and repository enrichment remains due because it has no provider
dependency. While no provider is configured, automatic query embedding and
graph walks ignore stored vectors and semantic-only evidence without deleting
them; an explicitly supplied valid raw query vector may still query the active
projection. Re-enabling the same model resumes that projection; enabling a
different model uses the controlled rebuild below.

### `edge_evidence`

The existing `edges` table remains the logical graph: one canonical edge per
source, relationship kind, and destination. `edge_evidence` stores one or more
reasons supporting that edge, including producer, evidence kind, confidence,
source revision hashes, model identity when semantic, and relevant chunks,
passages, paths, or symbols.

A logical edge exists while at least one current evidence row supports it.
Generated evidence is replaced by producer and source revision. Explicit
evidence is never overwritten by weaker semantic evidence. Generated symmetric
`relates_to` edges use lexically ordered typed endpoints and are traversed in
both directions.

### `entity_feedback` and scoped retrieval logs

Feedback aggregates and events use `(entity_kind, entity_id)` instead of
memory IDs in one table and code rowids in another. Retrieval logs store a
domain-scope bitset and typed result IDs under the single public `search`
source. Domain rankers still interpret feedback independently; the storage and
public command are unified.

Migration assigns stable opaque code entity IDs and converts rowid-based
feedback while the existing `code_symbols` rows still provide repository, path,
symbol, kind, and line identity. This happens before any code reindex.
Unresolvable historical rows are retained as orphaned telemetry and excluded
from ranking, not attached to a guessed symbol.

## Shared semantic projection

Semantic indexing is optional. With no configured embedder, memory saves,
document and code indexing, FTS, graph references, and unified lexical search
all remain successful. `doctor` and `watch status` report semantic enrichment
as disabled, not failed.

When an embedder is configured, saving a memory and indexing code or documents
create or refresh their semantic items automatically. The synchronous `index`
report distinguishes complete lexical indexing from pending enrichment;
semantic completion is not required for lexical success.

`COMEMORY_EMBED_CMD` becomes a versioned batch JSON command. Comemory invokes
the user-configured command once per bounded batch and writes one request to
standard input:

```json
{
  "version": 1,
  "inputs": [
    {"id": "batch-item-4", "text": "normalized text"}
  ]
}
```

The command must return exactly one embedding per input ID:

```json
{
  "version": 1,
  "model": "provider:model@revision",
  "dimensions": 3,
  "embeddings": [
    {"id": "batch-item-4", "values": [0.1, 0.2, 0.3]}
  ]
}
```

Comemory rejects duplicate, missing, non-finite, empty, or dimensionally
inconsistent results and normalizes accepted vectors for cosine search. Query
embedding uses the same protocol with one input. The default batch size is 64
and the default command timeout is 60 seconds. Batch IDs are opaque correlation
tokens and are not public entity IDs.

The response model string is authoritative. The effective expected model
identity—`COMEMORY_EMBED_HINT` overriding `semantic.model`—must match the
response when configured. An expected identity is required when callers supply
raw vectors through the retained `--vector` or `--vector-stdin` advanced
overrides, because raw vectors cannot self-identify. All such vectors target
the shared space and must match its locked dimension.

Changing the model identity or dimension invalidates only `semantic_vec` and
semantic edge evidence. Comemory recreates the vector table and queues all live
embedding units for regeneration. Lexical indexes, extracted content, explicit
edges, and lexical edges remain available during that rebuild. A partial batch
from a different model is never mixed into the active projection. A detected
model change initiates this controlled rebuild only during a mutating operation
(`save`, `index`, or watcher enrichment). A query response with a different
model drops its semantic leg, warns, and leaves the active projection unchanged.

The embedding command is an opt-in user integration, not a managed sidecar.
Comemory does not assume whether it runs locally or sends text elsewhere;
`doctor` warns that configured commands receive indexed text.

The surface-specific `serve --embed-cmd` and `tui --embed-cmd` flags are
removed. One configured provider serves every domain and surface. `save`,
`search`, and `context` retain their raw-vector overrides under the shared-space
rules above. The removed `index-code --extract` and `ingest-code` stream is not
replaced; automatic external-source embeddings flow through the configured
batch command.

Embedding input precedence is explicit raw `--vector`/`--vector-stdin`, then
`COMEMORY_EMBED_CMD`, then `semantic.embed_cmd` in `config.toml`, then
lexical-only operation. The configured command continues to execute through
`sh -c` and receives only the versioned JSON request. The old
raw-query/single-embedding output is diagnosed explicitly as "embedding
protocol v1 required"; the sample wrapper and documentation move to version 1
in the same release. Failure of an implicitly configured command falls back to
lexical query results with a warning and records pending enrichment during
indexing.

An invalid explicitly supplied raw vector remains a command error because
silently ignoring explicit input would conceal caller mistakes. A raw vector on
`save` initializes an empty semantic projection when the effective expected
model identity identifies its model; without that identity it is rejected. A
raw query vector requires an already initialized projection with the same model
and dimension; it never initializes or switches global semantic state.

## Automatic relationship engine

Relationships are refreshed after a file's lexical transaction commits. They
are built in three passes.

### Deterministic evidence

- Source-root and repository membership.
- Existing explicit memory relations and anchored file/symbol references.
- Markdown and rich-document hyperlinks whose local target resolves uniquely.
- Exact relative or repository-root file paths.
- Backticked or otherwise syntactically explicit code symbols that resolve to
  one indexed symbol.
- Normalized citations and URLs when their target is also indexed.
- Existing import and git co-change evidence from the code graph.

Ambiguous references remain unresolved diagnostics rather than guessed edges.

### Lexical evidence

- Exact normalized titles, identifiers, paths, and unique symbols when the
  normalized value occurs no more than three times in the indexed corpus.
- Exact content hashes and near-duplicate text using existing tokenization and
  SimHash distance. Near-duplicate matching reuses
  `rank.near_dup_hamming`, initially `8`.

Lexical candidates are ordered by exact hash, SimHash distance, evidence type,
and stable ID, then limited to reciprocal top-eight `relates_to` neighbors per
entity. This avoids duplicate cliques. The existing memory-only duplicate
advisory remains separate. Frequency counts and affected neighborhoods refresh
when entities change.

### Semantic evidence

The shared ANN index produces candidates across memories, documents, and code.
Document chunk and code-chunk candidates are first coalesced to stable entities.
Self-entity and same-document chunk matches are discarded.

For each entity, Comemory retrieves
`relationships.semantic_candidate_pool` candidates, keeps its best
`relationships.semantic_max_neighbors`, and emits a semantic `relates_to` edge
only when:

- cosine similarity is at least `relationships.semantic_min_similarity`; and
- each entity appears in the other's retained neighbor set.

Mutual-neighbor matching bounds every entity to the configured maximum and
suppresses generic hubs. The default knobs are
`relationships.semantic_min_similarity = 0.72`,
`relationships.semantic_max_neighbors = 8`, and
`relationships.semantic_candidate_pool = 24`. The best matching passage or
symbol and up to two supporting matches are recorded as evidence. Semantic
`relates_to` edges store their typed endpoints in lexical ID order and are
traversed bidirectionally; explicit directed relation kinds preserve their
direction.

Incremental reciprocal maintenance is deliberately not approximated. Each
vector deletion or successful vector batch advances
`semantic_corpus_generation`, invalidates the previous semantic-edge
generation, and schedules one debounced complete pass.
At start, that pass snapshots the corpus generation, semantic-model generation,
relationship-config generation, candidate pool, neighbor maximum, and
similarity threshold. It applies exactly that snapshot to every live entity and
writes a shadow evidence generation.

Activation is a compare-and-swap transaction: the shadow generation becomes
active only if all three current generations still equal the starting snapshot.
A vector write, model transition, or relationship-knob change during the pass
makes the comparison fail; Comemory discards the shadow rows and queues a new
debounced pass. Retrieval accepts semantic evidence only from the active tuple
matching the current corpus, model, and relationship-config generations while
continuing to use current vectors and all non-semantic evidence. Knob-only
changes increment the relationship-config generation but do not regenerate
vectors.

Explicit and structural evidence have greater graph and explanation weight
than lexical evidence; lexical evidence outranks semantic-only evidence.
Semantic failure never removes explicit or lexical relationships. Model or
source changes delete only the generated evidence whose provenance is stale.

`search --explain` and `edges --json` expose a relationship's strongest current
evidence, for example a direct link, matching path, near-duplicate passage, or
semantic passage/symbol pair. Structural containment and
source-membership edges remain available for filtering and explanation but are
excluded from default retrieval graph walks so source roots cannot become
ranking hubs. Explicit evidence retains a missing target as a ghost node for
explanation, but retrieval traversal never emits that ghost as a search result.

## Retrieval and ranking

Raw BM25 scores, vector similarities, and existing memory/code prior products
are not compared across domains.

Each domain produces an ordered typed candidate list:

- Memories retain the current lexical ladder, activation, feedback, quality,
  supersede, and rank priors.
- Code retains identifier-aware BM25, shared semantic candidates, PageRank,
  recency, working-set affinity, and feedback priors.
- Documents use passage BM25, shared semantic candidates, extraction
  confidence, freshness, and relationship rank.

The ranking sequence is fully ordered:

1. Within each domain, lexical and shared-ANN lists are fused with reciprocal
   rank fusion using the existing `retrieval.rrf_k`, default `60`. Domain priors
   then rerank that fused list. With no semantic query, the lexical list passes
   through unchanged.
2. The ANN results are split by domain before that rerank. Document and
   oversized-code chunks coalesce to their parent entity first.
3. The available domain lists are fused through a second RRF using the same
   `rrf_k`. An empty domain contributes no synthetic candidates.
4. The top `retrieval.graph_seeds` results, default `8`, seed the existing
   bidirectional graph walk up to `retrieval.graph_hops`, default `2`. The walk
   produces an ordered expansion list; it never contributes a raw score to the
   cross-domain list.
5. That graph list and the cross-domain list receive one final RRF with the same
   `rrf_k`. Typed ID is the deterministic final tie-breaker, then the existing
   pagination window is applied.

Evidence confidence is normalized to `[0,1]`. Its reliability multiplier is
`1.0` for authored/direct evidence, `0.9` for structural evidence, `0.8` for
lexical evidence, and `0.7` for semantic evidence. Multiple evidence rows for
one logical edge combine with noisy-OR:
`1 - product(1 - reliability * confidence)`. A graph candidate is ordered by
seed rank, the product of traversed logical-edge weights, a `0.8` per-extra-hop
decay, and typed ID. Source membership/containment and ghost targets are
ineligible for this walk.

The final JSON `score` is the last RRF score. `score_parts` exposes each
domain-list rank, graph-list rank, RRF contribution, and the domain priors that
ordered the candidate; it does not present incomparable BM25 or cosine values
as a common relevance score.

Low-confidence extraction is a bounded demotion, not an exclusion: normalized
confidence in `[0,1]` maps to a multiplier of `0.75 + 0.25 * confidence`. A
retained last-good revision is searchable with `stale: true` and receives a
`0.85` freshness multiplier. TTY output labels stale results; JSON includes the
failure and last-success timestamps when requested through score details.

## Index lifecycle and atomicity

For each suspected file change:

1. Verify that the source is reachable and the path is in bounds.
2. Compare size and modification time.
3. Hash the file only when the fast fingerprint changed or reconciliation
   requires confirmation.
4. Skip extraction when the content hash is unchanged.
5. Extract outside a database transaction.
6. Recheck the file fingerprint.
7. In one SQLite transaction, replace the file's document/code rows, FTS rows,
   current semantic-item metadata, and revision-bound relationship evidence.
8. Queue embeddings and derived relationships that were not available in the
   lexical transaction.

A failed update leaves the previous good revision intact, marks it stale, and
records the new error. A failed first extraction records the source file and
diagnostic but creates no searchable content. A successful retry atomically
replaces the stale revision.

Deletion is applied only when the registered source is reachable and absence
is confirmed by an authoritative scan or a validated watcher event. Deletion
removes derived rows and generated evidence for that entity. External files are
never touched.

Confirmed watcher rename pairs retain the document ID. When events are
insufficient, an unchanged content hash may establish a rename only if it is
unique within the source. Otherwise the operation is conservatively treated as
delete plus create.

## Watcher and lazy reconciliation

There is at most one watcher per canonical Comemory data directory. It is the
same binary running a hidden internal command. Public lifecycle is managed
through `watch start|stop|status` using an advisory lock, process identity
token, PID, and local control endpoint. Stale PID state never establishes
liveness by itself.

Lifecycle commands are idempotent. `watch start` does not launch a process when
no registered source uses `watch` and no background-only enrichment is pending;
it reports an idle configuration instead. Pending repository or semantic jobs
may therefore keep the coordinator useful even when every source is lazy.
`watch stop` succeeds when no watcher is running. A watcher whose executable
version or process-identity token does not match the invoking binary is reported
as incompatible and must be stopped before the new watcher starts.

The registry contains one durable global `watcher.autostart` setting in addition
to the per-source refresh modes. It defaults to `true`. `watch start` sets it to
`true` before launching; `watch stop` stops gracefully and sets it to `false`
without changing any source's `watch` mode. This records an operator's deliberate
stop across later commands and process restarts. It is desired operational state,
not liveness: PID, queue, and heartbeat data remain ephemeral. Autostart means
that the next normal Comemory command ensures the same-binary watcher exists; it
does not install an operating-system service.

After its synchronous initial index, any `index` operation whose effective
refresh mode is `watch` sets autostart to `true` and ensures the watcher is
running. An `index` that only registers lazy sources leaves the global setting
unchanged. Every read of derived external state calls the same
`ensure_fresh(scope)` preflight: CLI `search`, `context`, `edges`, and `graph`;
TUI searches; and `/api/search` and `/api/graph` in `serve`. The preflight
behaves as follows:

1. Lazy sources in scope receive a synchronous fingerprint reconciliation.
2. For watch sources with autostart enabled, it health-checks the watcher. A
   missing or crashed watcher receives one restart attempt, subject to the same
   persisted retry backoff used for root failures.
3. A healthy watcher accepts a scoped barrier and drains all filesystem events
   observed before that barrier. If it cannot accept or complete the barrier,
   the caller synchronously reconciles the affected watch sources.
4. When autostart is disabled, the caller does not restart the watcher and
   synchronously reconciles watch sources instead.
5. The caller spends only the bounded inline-safe enrichment budget defined
   above. Remaining or failed enrichment returns the valid last-good lexical
   state with typed pending or degraded diagnostics; it never makes a
   known-stale result look current.

Every registry mutation (`index`, `unindex`, a source-mode update, `watch
start`, or `watch stop`) serializes through the registry and index locks and
increments the registry generation in the same atomic TOML replacement. While
those locks are held, the command reconciles the SQLite mirror. It releases the
locks before sending the running watcher a full-registry reload carrying that
generation, so acknowledgement cannot deadlock on registry access. The watcher
validates and atomically swaps its snapshot before acknowledging the generation.
An acknowledgement timeout is reported as degraded watcher health; the TOML
remains authoritative and lazy fallback remains available.

Every queued watcher event carries the generation and source ID from the
snapshot that admitted it. Lock order is always registry then index. Immediately
before a file transaction commits, the writer takes the registry lock in shared
mode, reads the authoritative TOML generation, and compares it with both the
event and the completely mirrored SQLite generation. If the mirror lags—for
example, because a command crashed after replacing TOML—it is reconciled before
any event may proceed. A removed source, changed boundary, or stale generation
makes the event ineligible; it is discarded or rediscovered from the new
snapshot. The watcher also treats a filesystem change to `sources.toml` as an
immediate reload signal. These commit-time checks prevent an in-flight event
from recreating rows after `unindex`.

`watch status`, `sources`, and `doctor` are observational and never start a
watcher or reconcile content. `serve` and the TUI do not maintain a separate
freshness implementation: each query passes through the shared preflight, so a
watcher crash after either process starts has the same fallback behavior as a
CLI query.

The watcher pipeline is:

```text
filesystem events
  -> source/path validation
  -> 500 ms debounce and path coalescing
  -> bounded extraction queue
  -> extraction workers
  -> single index writer
  -> enrichment queue
```

Atomic-save event sequences collapse into one logical update. A queue overflow,
watch backend error, or ambiguous rename schedules a complete scan of the
affected source. A periodic reconciliation scan catches missed events and
unreliable network filesystem behavior.

The defaults are `indexing.watch_debounce_ms = 500`,
`indexing.watch_queue_capacity = 1024`, and
`indexing.reconcile_interval_secs = 900`. Root retry starts at one second,
doubles with plus-or-minus 20 percent jitter, and caps at five minutes.

SQLite remains in WAL mode for concurrent readers. A per-data-directory index
lock prevents a CLI lazy scan and the watcher from mutating the same file at
the same time. Work is deduplicated by source ID, relative path, and observed
fingerprint.

If a mount or entire root is temporarily unavailable, all of its indexed
entities remain present and become stale. The watcher applies exponential
retry and does not interpret an unavailable root as mass deletion.

`watch status`, `sources`, and `doctor` expose watcher liveness, queue depth,
last event and reconciliation times, per-status counts, last-good revisions,
extraction failures, missing OCR packs, semantic state, and pending semantic
rebuilds.

Repository-level import resolution, co-change mining, PageRank, and
reinforcement run after the affected repository's file transactions. Separate
lexical and enrichment generation counters make a failed post-pass visible and
retryable without re-extracting unchanged files.

## Configuration transition

The layered config parser must genuinely accept the new `[indexing]`,
`[semantic]`, `[relationships]`, `[ocr]`, and `[extraction]` keys described in
this specification; documentation alone is not sufficient. Environment values
override `config.toml`, and command flags override both where a flag is defined.

The complete set of new or changed knobs for this feature is:

| Key | TOML type | Default | Validation and meaning |
| --- | --- | --- | --- |
| `indexing.refresh` | string enum | `"watch"` | `watch` or `lazy`; default for newly registered sources |
| `indexing.watch_debounce_ms` | integer | `500` | `50..=60_000` |
| `indexing.watch_queue_capacity` | integer | `1_024` | `1..=1_000_000` paths |
| `indexing.reconcile_interval_secs` | integer | `900` | `1..=86_400` |
| `indexing.tombstone_retention_days` | integer | `30` | `1..=3_650` |
| `indexing.query_enrichment_budget_ms` | integer | `25` | `0..=1_000`; zero disables query-inline enrichment |
| `indexing.query_enrichment_max_jobs` | integer | `32` | `0..=1_024`; zero disables query-inline enrichment |
| `extraction.max_file_bytes` | integer | `104_857_600` | greater than zero |
| `extraction.max_pages` | integer | `5_000` | `1..=1_000_000` |
| `extraction.timeout_secs` | integer | `120` | `1..=86_400`, per file |
| `extraction.max_worker_rss_bytes` | integer | `1_073_741_824` | greater than zero |
| `extraction.max_output_bytes` | integer | `268_435_456` | greater than zero |
| `extraction.max_chunks` | integer | `100_000` | `1..=1_000_000`, per file |
| `extraction.workers` | integer | `0` | `0..=256`; zero selects `max(1, min(4, available_parallelism))` |
| `semantic.embed_cmd` | string or absent | absent | nonempty shell command when present |
| `semantic.model` | string or absent | absent | nonempty expected response model identity when present |
| `semantic.batch_size` | integer | `64` | `1..=4_096` |
| `semantic.timeout_secs` | integer | `60` | `1..=3_600`, per batch or query |
| `relationships.semantic_min_similarity` | float | `0.72` | finite and in `[0, 1]` |
| `relationships.semantic_max_neighbors` | integer | `8` | `1..=256` |
| `relationships.semantic_candidate_pool` | integer | `24` | `1..=4_096` and not less than `semantic_max_neighbors` |
| `ocr.download_models` | Boolean | `true` | enables checksum-pinned language-pack downloads on demand |
| `ocr.languages` | string array | `[]` | unique nonempty Tesseract language codes; empty selects automatic detection |
| `rank.near_dup_hamming` | integer | `8` | existing key, now also used by cross-domain near-duplicate evidence; `0..=64` |

Every scalar new key maps to an uppercase
`COMEMORY_<SECTION>_<KEY>` override except the semantic provider keys, which
retain the deliberate public names `COMEMORY_EMBED_CMD` and
`COMEMORY_EMBED_HINT`. `COMEMORY_OCR_LANGUAGES` is a comma-separated list;
an empty value means the empty automatic-selection list. File and environment
inputs share the same validation, and unknown keys remain errors.

The old top-level `embed_hint` key is rejected with an actionable instruction
to move it to `[semantic] model`; it is not silently treated as a second model
source. Existing `COMEMORY_EMBED_HINT` remains valid and overrides that new key.
The old `[indexing]` keys `auto_reindex`, `auto_reindex_threshold_ms`, and
`incremental_batch_size` are rejected with replacement guidance rather than
ignored. The reporting-only split `embeddings.memory_model` and
`embeddings.code_model` settings are removed because one active shared model is
recorded from the batch protocol.

If the removed `COMEMORY_INDEXING_AUTO_REINDEX` variable is present, startup
returns an actionable configuration error rather than silently ignoring it.
The message maps old `lazy` to the new synchronous `lazy`, old `hook` to
`watch` plus managed-hook cleanup, and explains that old manual-only `off` has
no registered-source equivalent. The new lazy mode deliberately reconciles
before returning search results instead of launching a detached update and
returning stale results.

## Safety and privacy

- All document paths are canonicalized and checked against their registered
  boundary before open and again before commit.
- Archive extraction is disabled, and MIME/signature validation prevents
  extension spoofing from enabling it.
- Parser work is constrained by subprocess time and memory limits.
- OCR model downloads use HTTPS, pinned versions, and checksums; they can be
  disabled. Normal indexing and tests otherwise require no network.
- No source text leaves Comemory unless the user explicitly configures an
  embedding command whose own behavior does so.
- Source text, OCR output, and vectors remain in the selected Comemory data
  directory. Passwords are never requested or stored.
- Extraction diagnostics avoid including full document passages by default.

## Migration and rebuild

The migration is a clean public-interface break but a conservative data
migration. Migration and rebuild first acquire the per-data-directory
maintenance lock and quiesce the watcher and index writers. They publish no
schema or replacement-database work until quiescence is confirmed. The one
exception is the deliberately persistent downgrade fence in the first-upgrade
protocol below; it may commit before a second legacy-client scan precisely to
make a racing old open fail safely.

The first breaking upgrade has an additional legacy-client fence because an
already-running old TUI or `serve` process does not know about the new
maintenance lock. Before changing the schema, the new binary enumerates
same-user processes with an open handle to the database, WAL, or shared-memory
file—`/proc` file descriptors on Linux, `libproc` on macOS, and Restart Manager
on Windows. Any handle owned by another PID aborts with that PID and shutdown
guidance. If the platform cannot prove handle ownership, migration fails closed;
there is no force bypass.

After a clean scan, an exclusive SQLite transaction first completes and
validates every migration marker known to the last legacy schema, then installs
a permanent schema-version downgrade guard, records the new numeric version
with `upgrade_state = 'fenced'`, and commits before destructive work. Therefore
any older binary finds all of its own migration markers already complete and
must reach its final version write; the guard rejects that lower version before
the command can serve a request. The new binary repeats the open-handle scan
after the fence; this catches a legacy process that raced the first scan, while
the committed guard prevents any later one from opening successfully. A fenced
but incomplete database is opened only by the new binary's recovery migrator,
which resumes from the recorded state. This protocol is specific to the first
upgrade; generation-aware clients use the maintenance lock thereafter.

1. Open the old database through a raw, read-only connection that does not run
   automatic migrations. Snapshot repository labels, recorded roots, identity
   paths, code-rowid mappings, feedback, learning state, and explicit edges
   while the legacy schema is still readable.
2. Build an atomic `sources.toml` candidate from unambiguous repository-root
   metadata, retaining the exact old label and path base. Paths that cannot be
   recovered are reported by `doctor` and require an explicit
   `comemory index <path>`; Comemory does not guess from a repository label.
   The candidate is not published until the schema transaction succeeds.
3. In one SQLite migration transaction, add the source/document/evidence and
   typed-feedback tables, preserve authored memories and durable learning data,
   translate rowid-based code feedback, and convert explicit logical edges to
   evidence-backed rows.
4. Remove the incompatible legacy memory/code vector tables and mark the shared
   semantic projection for regeneration. Derived import, co-change, rank, FTS,
   and semantic evidence is regenerated rather than treated as authoritative.
5. Reindex recoverable registered code and documents through the unified
   pipeline, then restart the watcher when registry autostart is enabled and at
   least one source's desired mode is `watch`.

Legacy code whose root cannot be recovered remains lexical-only through the
synthetic stale file records defined above, with preserved label/path identity
and an actionable registration diagnostic. It is adopted or replaced when the
user registers the correct source; migration does not discard it merely because
an earlier rebuild lost `repo_marker.root_path`.

Before the fence commits, any failure leaves the prior schema and binary usable.
After the fence, a failed schema transaction rolls back all schema/data work but
deliberately keeps the downgrade guard: the physical legacy data remains intact,
the old binary is rejected, and the new binary reports and retries the fenced
migration. The registry candidate is not published and the watcher remains
stopped until that retry succeeds. A successful in-place schema migration
clears `upgrade_state`, creates a missing `schema_meta.database_uuid`, initializes
or increments `schema_meta.database_generation` in the migration transaction,
and causes generation-aware long-lived processes to reopen their connections.

`comemory rebuild` reconstructs a replacement database from managed memory
Markdown plus `sources.toml` and re-extracts reachable external files into a
temporary database. Every database operation in every process holds the shared
side of the per-data-directory maintenance lock. Rebuild takes the exclusive
side, checkpoints the old WAL, pauses the watcher, and waits for active CLI,
TUI, and `serve` operations to drain before publishing anything.

`schema_meta` contains a generated `database_uuid` and monotonically increasing
`database_generation`. When the existing database is readable, the temporary
database preserves the UUID and uses generation plus one. After integrity and
schema validation, rebuild uses SQLite's backup API to copy the temporary source
into the existing destination under the exclusive application lock. The backup
destination transaction is the publication boundary: failure rolls it back
before the lock is released. This keeps the live database path and inode stable
and avoids a rename/token crash window or platforms where idle handles block
replacement.

After acquiring the shared maintenance lock, each connection manager reads the
UUID and generation before checkout. A mismatch with its cached pair discards
prepared statements and the entire pool, then opens and validates fresh
connections before continuing. Watcher, CLI, TUI, and `serve` all use this
store seam. No process can begin an operation between publication and the
generation check, and there is no external generation token that can disagree
with the database.

If the target database is absent or too damaged to open, there can be no valid
live readers to preserve. Rebuild then uses an fsynced, checksum-bearing swap
journal that names only the exact target, temporary database, recovery copy,
and new database UUID. It renames in that order with a directory `fsync` after
each step. Startup detects the journal, validates both candidate databases, and
deterministically completes the publish or restores the recovery copy before
opening the store. The journal is removed after a successful directory `fsync`.
This is a short-lived crash-recovery record, not a runtime helper or service.

When the prior database is readable, rebuild preserves feedback aggregates and
events, retrieval logs, query expansions, bandit/tuning state, explicit memory
relations, learned `co_activated` evidence, source identities, document/code
entity IDs, and last-good extracted content. FTS rows, vectors, rank scores,
imports, co-change edges, and other reproducible indexes are regenerated.
When a registered source is unreachable or re-extraction fails, its copied
last-good lexical content remains searchable as stale. Synthetic legacy
source-file rows and their code entities are copied when the prior database is
readable, remain non-editable, and retain converted references and feedback;
their count and adoption guidance are reported.

Existing live document and code IDs are reused through source/path, symbol, and
unique content-hash matching when the old database is available; no promise is
made to preserve an otherwise unrecorded identity after both the database and
path have been lost. If no readable prior database exists, rebuild can recover
only managed Markdown and reachable registered sources.

Generated CLI documentation, shell completions, configuration documentation,
examples, the sample embedding wrapper, and Git hook guidance are updated in
the same change. Obsolete hook instructions are removed.

The implementation inventory explicitly includes `README.md`, `CLAUDE.md`, the
architecture/configuration/getting-started and affected guide pages,
`scripts/regen-cli-docs.sh`, `scripts/e2e.sh`, `scripts/comemory-embed.sh`, CI
and `.cargo/config.toml` references to old environment settings, dimensioned
benchmarks, nextest groups, and the command-mirrored integration tests. Existing
historical changelog entries remain historical and are not rewritten.

Source reconciliation becomes the owner of deleted and stale external code, so
`prune` no longer performs a separate stale-code scan. Authored memory
references to a missing external target remain as dangling explicit evidence;
generated evidence owned by the missing target is removed. Memory pruning
retains its current behavior. `gc` purges `deleted` source-file tombstones and
resolved diagnostics after `indexing.tombstone_retention_days`, initially 30.

## Error behavior

Errors are isolated to the smallest useful unit:

- One corrupt file does not fail the source index.
- One failed source does not prevent other sources from reconciling.
- One embedding batch failure leaves lexical content searchable and is retried.
- One relationship producer failure leaves other evidence and the logical edge
  intact.
- A database transaction failure leaves that file's previous revision intact.
- A watcher failure activates synchronous lazy reconciliation for retrieval.

Failure to auto-start the watcher after a successful `index` is reported as a
degraded partial success and leaves the source's desired mode as `watch`; the
lazy fallback remains active. An explicit `watch start` failure returns
nonzero.

`index` returns nonzero only for command-level failures such as an invalid root,
registry write failure, or unavailable database. Per-file failures produce a
successful partial-index report with error counts; `--strict` upgrades any
per-file error or watcher-start degradation to a nonzero exit after all files
have been attempted. JSON output always distinguishes command failure, partial
success, and complete success.

## Testing strategy

Tests remain flat under `tests/` and mirror every new `src/` module according to
the project binding rules.

### Unit and component tests

- Classification precedence, MIME mismatch, ignore files, source containment,
  and nested-root rejection.
- Source-registry atomic writes, inferred-label collisions, migrated identity
  bases, generation increments, watcher acknowledgements, and SQLite mirror
  reconciliation.
- Document normalization, structural chunking, table splitting, overlap, and
  provenance retention.
- Fingerprint/hash decisions and every file-status transition.
- Batch embed protocol validation, normalization, model/dimension locking, and
  model-change invalidation.
- Typed candidates, document coalescing, per-domain ranking, cross-domain RRF,
  typed filtering, opaque-ID stability, and duplicate-symbol disambiguation.
- Deterministic, lexical, and reciprocal semantic edge generation; evidence
  replacement, generation-CAS rejection during concurrent vector/config
  changes, and edge lifetime.
- Every new configuration default and boundary, file/environment precedence,
  cross-field validation, and actionable rejection of removed keys.
- Synthetic legacy file-ID stability, unambiguous adoption, v2 file-API edit
  tokens, and rejection of editing stale/document/orphan files.
- Query enrichment time/job budgets, deadline cancellation, and proof that
  embedding, repository-wide, and semantic-corpus jobs never run inline.
- Typed feedback/log migration while legacy code rowids are still resolvable.

### Extraction fixtures

Checked-in fixtures cover TXT, Markdown, HTML, DOCX, native-text PDF,
scanned-image PDF, mixed native/OCR PDF, locked PDF, corrupt document,
oversized table, and a mixed code/document repository. OCR expectations use
bundled English data and never download during tests.

Worker tests cover malformed JSON, crash, timeout, memory-limit termination,
source mutation during extraction, and successful retry while retaining the
last good revision.

### Watcher and lifecycle tests

Deterministic coordinator tests inject create, modify, atomic replace, rename,
delete, duplicate events, queue overflow, backend failure, unavailable root,
and restart. Platform integration tests exercise the real watcher backend with
bounded polling rather than fixed sleeps. Tests persist `watch start` and
`watch stop` intent across commands, prove an effective watch-mode `index`
reenables autostart, and exercise the same scoped freshness barrier through CLI,
TUI, and `/api/search`. Lazy-mode, explicitly stopped, crashed, and incompatible
watcher cases must reconcile before returning without an unintended restart.
Maintenance tests quiesce the watcher around migration/rebuild and prove
watcher, TUI, and serve connection pools reopen after a
`schema_meta.database_generation` change. Crash injection at every backup and
unopenable-database swap-journal boundary must leave startup able to select one
integrity-checked database deterministically.

A registry race test pauses an admitted watcher event, commits `unindex`, then
proves the stale-generation event cannot recreate the source. Cross-version
tests hold the legacy database open through the current pre-upgrade `serve` and
TUI connection patterns and require the first-upgrade scan to abort. After
those handles close, migration must fence successfully; a legacy open then
fails at the downgrade guard, while the new binary completes or resumes a
crash-injected fenced migration.

### Retrieval and end-to-end tests

A deterministic fake batch embedder creates known cross-domain neighbors and
model-change scenarios. End-to-end CLI tests cover mixed-root indexing,
unified search and filters, citations, feedback, unindexing, watcher status,
lazy refresh, extraction failure recovery, fully lexical operation, removed
command guidance, stale managed-hook detection, old-config diagnostics,
memory-only time filters, and the context schema-version-2 envelope.

Golden retrieval fixtures use typed relevant IDs and assert recall/MRR floors
for memory, document, code, and mixed queries. Ranking snapshots include the
relationship explanation and stale/confidence behavior.

Normal test execution requires no network. The required gates are:

```text
just check
just test
just qa
just e2e
```

Dependency and license checks cover the exactly pinned Xberg and statically
linked OCR stack.

## Delivery sequence

Implementation should land as four reviewable vertical increments, each
leaving lexical Comemory usable:

1. Source registry, classifier, unified CLI, schema, and lexical TXT/Markdown
   document indexing.
2. Xberg adapter, isolated workers, rich formats, structural chunks, and OCR.
3. Shared semantic projection, unified retrieval, feedback, and automatic
   relationships.
4. Watcher, lazy fallback, reconciliation, migration/rebuild integration,
   diagnostics, documentation, and hardening.

The implementation plan may split these increments into smaller tasks, but it
must preserve the module-size, no-warning, mirrored-test, and no-duplication
rules in `CLAUDE.md`.

## Acceptance criteria

The feature is complete when:

- one installed `comemory` executable indexes a mixed software/document root;
- PDF, DOCX, TXT, and Markdown fixtures are searchable with reliable citations;
- `search` returns memories, documents, and code through typed unified results;
- deterministic relationships work without embeddings and explain their
  evidence;
- one configured embedding model produces cross-domain semantic retrieval and
  reciprocal, degree-bounded relationships;
- watcher mode updates changed files automatically, and lazy mode or watcher
  failure reconciles before retrieval on CLI, TUI, and API surfaces;
- parser, OCR, embedder, and watcher failures preserve the last good searchable
  revision and produce actionable diagnostics;
- retrieval never executes queued external or corpus-wide enrichment inline,
  bounds its single query-embedding call, and reports work that remains durably
  pending;
- model changes rebuild only semantic projection and semantic evidence;
- stale watcher generations cannot recreate an unindexed source, and the first
  breaking migration refuses live legacy database clients;
- `unindex` and rebuild never modify external source files;
- no external extraction service, sidecar, Java runtime, Python runtime, or
  office suite is required; and
- all project quality gates pass without network-dependent tests.
