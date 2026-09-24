# comemory documentation

`comemory` fuses engram-style developer memory, semantic code search, and
ast-grep AST patterns into one local SQLite-backed CLI. These docs are
organized by what you're trying to do.

## Rendered site

The same pages render at [falconiere.github.io/comemory](https://falconiere.github.io/comemory/):
`docs/` doubles as the GitHub Pages root. `index.html` is the landing page,
`architecture.html` walks the design as interactive cards, and
`documentation.html` renders every markdown file here in the browser (the
`.nojekyll` marker keeps the `.md` files served raw so nothing is copied).
The site is set in the Signal design language from
[toolu-conventions](https://github.com/Falconiere/toolu-conventions/blob/main/DESIGN.md);
its tokens live in `assets/signal.css`.

## Start here

- **[Getting started](getting-started.md)** — install, save your first memory,
  search, and index code in a few minutes. Read this first.

## How-to guides

Task-oriented recipes for a specific job:

- **[Agent skills and hooks](guides/agent-integration.md)** — install the Claude
  Code or Codex integration and migrate from toolu.
- **[Bring your own vectors](guides/byo-vectors.md)** — embed memories and code
  with your own model via `--vector` / `--vector-stdin` (dims 1024 / 768).
- **[Keep the code index fresh](guides/auto-reindex.md)** — `lazy` (default),
  `hook`, and `off` auto-reindex modes and how the wired lazy trigger works.
- **[Measure and tune ranking](guides/ranking-and-eval.md)** — the
  `eval → mine → tune` learning loop and the ranking knobs.
- **[The HTTP API](guides/http-api.md)** — the versioned `/api/v1` REST
  surface mirroring the CLI: envelope, auth, route map, jobs, and gating.
- **[Cloud sync](guides/cloud-sync.md)** — `auth` / `sync` against
  comemory.io: log in once and the first sync runs itself; organization
  membership decides what leaves the machine.
- **[Replication harness](guides/replication-e2e.md)** — real-process
  convergence cases against a platform checkout, and the coverage gate.
- **[Session capture](guides/session-capture.md)** — redact a coding-tool
  transcript locally and post a Slice 3 receipt (`capture session` /
  `capture sources` / `capture install-hook`).
- **[Link code to memories](guides/linking-code-to-memories.md)** — pin
  `--ref-file` / `--ref-symbol` references and read fresh/stale/ghost status.
- **[Prune, rebuild, and gc](guides/prune-and-gc.md)** — maintenance: trim
  low-value memories, rebuild the DB from markdown, garbage-collect logs.
- **[Upgrading comemory](guides/upgrading.md)** — what a schema upgrade does,
  where snapshots go, how to restore one, and the `serve`-restart caveat.
- **[Schema migrations](guides/schema-migrations.md)** — for contributors:
  the declared `#[table]` schema, `just migration <name>`, hand-SQL tables,
  and the `migrations/` journal.
- **[Runtime queries](guides/runtime-orm.md)** — toolu-orm builders, execution
  behavior, and the capability inventory for SQL awaiting upstream support.

## Reference

Look-it-up material:

- **[CLI reference](cli-reference.md)** — every subcommand and flag, with the
  `--json` pagination envelope (generated from `--help`). Includes
  [`comemory auth`](cli-reference.md#comemory-auth) (device
  login; CLI-only).
- **[CLI + HTTP scenario catalog](scenarios/README.md)** — the human-readable
  test plan: every subcommand, every flag, its `/api/v1` twin, and the test
  that covers it ([auth](scenarios/auth.md)).
- **[Configuration](configuration.md)** — every environment variable (including
  `COMEMORY_API` / `COMEMORY_API_KEY`), the config-file-only knobs, and the
  pagination envelope shape.
- **[Release process](release.md)** — how releases are cut and published.
- **[Container image](container-image.md)** — the multi-arch image published to
  GHCR on every tag: tags, mounts, and running as another user.
- **[Build performance](build-perf.md)** — build-time notes.

## Explanation

Understanding-oriented background:

- **[Architecture](architecture.md)** — the design: storage layout, the
  retrieval pipeline, the edge graph, auto-reinforcement, and pagination.
- **[Sync daemon design](designs/2026-09-14-sync-daemon.md)** — user-level
  OS daemon + exhaustive post-login sync (replaces in-process auto-sync hooks).
- **[Replication harness](designs/2026-09-21-replication-e2e-harness.md)** —
  the real-process cases and coverage gate for cross-machine sync. The
  how-to is [replication-e2e](guides/replication-e2e.md).
- **[Replica-v1 journal](designs/2026-09-21-replica-v1-journal.md)** — the
  versioned replication journal, immutable payloads, acceptance receipts,
  server-ordered sequences and epoch-bearing cursors behind
  `/api/v1/sync/replica/*`.
- **[Memory mutation capture](designs/2026-09-22-memory-mutation-capture.md)** —
  which memory writers owe a replication operation and which deliberately owe
  none, the write intent that makes an interrupted write recoverable, and the
  embedding that rides outside the revision digest.
- **[Code generation replication](designs/2026-09-22-code-generation-replication.md)** —
  what one machine learned about a repository's shape, carried to another as
  an immutable, content-addressed generation that becomes visible whole: why
  source never leaves, why a pulled generation never writes a local row, and
  which readers see both sides.
- **[Document revision replication](designs/2026-09-23-document-revision-replication.md)** —
  what one machine extracted from a document, carried to a machine that may not
  hold the file: the portable name two checkouts agree on, the approval map that
  makes it answerable offline, the secret scan that blocks a revision whole, and
  one search leg over both indexes with the local side winning.
- **[Feedback and activity replication](designs/2026-09-24-feedback-activity-replication.md)** —
  verdicts and command runs as immutable `replica-v1` events counted exactly
  once: the event id and device every one carries, the namespaced query id
  that keeps an import out of this machine's recall loop, what is shared and
  what stays local (the summary allowlist, machine paths, secrets,
  co-activation rewards), and retention and purge reaching the journal.
- **[Domain-first migration contract](designs/2026-09-17-domain-first-migration.md)** —
  staged, behavior-preserving migration of the Rust CLI from technical layers
  to business capabilities.
- **[Reranker command protocol](designs/2026-09-18-reranker-command-protocol.md)** —
  the versioned JSON stdin/stdout contract for an optional external relevance
  scorer and the deadline-bounded process runner that carries it. Its
  § Wire protocol section is the field-by-field contract; nothing on the search
  path calls it yet.
- **[Benchmarking token efficiency](benchmark.md)** — what exists to measure
  how many tokens and tool calls comemory saves an agent, and the first
  experiment that would put a number on it.
- **[MCP transport and learning-loop hooks](designs/2026-09-18-mcp-transport-and-learning-loop.md)** —
  `comemory mcp`, the stdio adapter beside `cli` and `serve`, its curated
  eleven-tool catalog, and the recall-injection / Stop-enforcement / SessionEnd
  hooks that close the learning loop.
- **[Real-time activity feed](designs/2026-09-20-activity-feed.md)** —
  `activity_log`, one row per agent-visible command run, and the two routes
  that read it: `GET /api/v1/activity` and its SSE twin. Recording happens at
  the command core, so `cli`, `serve` and `mcp` each count once.