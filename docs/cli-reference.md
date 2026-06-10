# CLI reference

This page is **generated** by `scripts/regen-cli-docs.sh`. Do not edit by
hand — re-run the script and commit the result. Drift is enforced by
`scripts/cli-docs-check.sh` in the umbrella gate.

## Global options

Every subcommand inherits two global flags:

| Flag | Description |
|---|---|
| `--json` | Emit machine-readable JSON instead of a human TTY view. |
| `--data-dir <DATA_DIR>` | Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable. |

Exit codes follow `sysexits.h`: `0` success, non-zero for usage / I/O /
data errors.

## Top-level help

```
Agentic dev memory + code-aware semantic search

Usage: comemory [OPTIONS] <COMMAND>

Commands:
  save           Save a memory (body via arg, `-`, or stdin)
  search         Search the memory index by natural-language query
  list           List memories with optional repo/kind filters
  delete         Soft-delete a memory by id (moves to `.trash/`)
  feedback       Record per-memory feedback (used vs irrelevant)
  doctor         Report on the data directory and SQLite mirror health
  index-code     Walk a repo, extract symbols, and upsert into the code index
  ingest-code    Read pre-embedded JSONL rows from stdin and ingest them into the code index (`code_symbols` + `code_fts` + `code_vec`)
  ast            Run an ast-grep pattern against a single source file
  context        Headline lookup: code symbol + memories matching a key
  completions    Emit a shell completion script for `bash`, `zsh`, `fish`, `powershell`, or `elvish`
  prune          Detect (and optionally soft-delete) stale memories
  rebuild        Drop `comemory.db` and repopulate it from the markdown source of truth
  gc             Purge old entries from `memories/.trash/`
  install-hooks  Install git hooks that trigger `comemory index-code --incremental` on `post-commit`, `post-merge`, and `post-checkout`
  help           Print this message or the help of the given subcommand(s)

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help
  -V, --version              Print version
```

---

## comemory save

```
Save a memory (body via arg, `-`, or stdin)

Usage: comemory save [OPTIONS] [BODY]

Arguments:
  [BODY]  Memory body. Use `-` (or omit) to read from stdin

Options:
      --json                     Emit machine-readable JSON instead of a human TTY view
      --kind <KIND>              Memory kind: decision|bug|convention|discovery|pattern|note [default: note] [possible values: decision, bug, convention, discovery, pattern, note]
      --data-dir <DATA_DIR>      Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --repo <REPO>              Repo name attached to the memory (free-form string) [default: ""]
      --tags <TAGS>              Comma-separated tag list (e.g. `database,postgres`) [default: ""]
      --author <AUTHOR>          Author identifier. Defaults to empty so callers may omit [default: ""]
      --quality <QUALITY>        Quality rating 1..=5. Defaults to 3 [default: 3]
      --supersedes <SUPERSEDES>  Comma-separated 8-hex memory ids this memory replaces (e.g. `a1b2c3d4,e5f6a7b8`). Recorded in the frontmatter `relations.supersedes` list and materialized as `supersedes` edges, so the older memories are demoted in ranking and annotated `superseded_by` in search results [default: ""]
      --vector <VECTOR>          Caller-supplied dense vector as a comma-separated float list. Length must equal the configured memory vector dim or the save fails with `vector dim mismatch`
      --vector-stdin             Read a JSON `{ "embedding": [..] }` payload from stdin and use it as the dense vector for the saved memory. Mutually exclusive with body being read from stdin (the body must be supplied as a positional arg when `--vector-stdin` is set)
  -h, --help                     Print help

Examples:
  # Save a decision with tags and elevated quality
  comemory save "Use Postgres for analytics" --kind decision --repo myrepo --tags db,postgres --quality 4

  # Pipe a bug report body from another command
  echo "Race in run_migration when run twice in <1s" | comemory save - --kind bug --repo myrepo

  # Save with a caller-supplied embedding (BYO-vector)
  echo '{"embedding":[0.1,0.2,...]}' | comemory save "...body..." --vector-stdin

  # Minimal note (kind defaults to `note`, no repo/tags)
  comemory save "Remember: cargo nextest serializes the embedder group"

  # Replace an outdated memory: a1b2c3d4 is annotated `superseded_by` in
  # search results and demoted in ranking (score_parts.supersede = 0.2)
  comemory save "new convention: pgbouncer in transaction mode" --supersedes a1b2c3d4

  # Near-duplicate detection: if a similar memory exists, a TTY warning is
  # printed to stderr and --json output includes a `duplicate_of` field with
  # the matching memory id. The save always proceeds — use `--supersedes` to
  # mark the relationship if the new memory replaces the old one.
```

---

## comemory search

```
Search the memory index by natural-language query

Usage: comemory search [OPTIONS] <QUERY>

Arguments:
  <QUERY>  Natural-language query string

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --k <K>                Override the configured `retrieval.top_k`. Must be >= 1
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --repo <REPO>          Optional repo filter forwarded to the vector branch
      --vector <VECTOR>      Caller-supplied dense vector as a comma-separated float list
      --vector-stdin         Read a JSON `{ "embedding": [..] }` payload from stdin and use it as the dense vector for the query
  -h, --help                 Print help

Examples:
  # Natural-language query, top 12 hits (default); weighted BM25 + priors
  comemory search "postgres pool exhausted"

  # Identifier-aware matching — camelCase/snake_case tokens split automatically
  comemory search "VecDimMismatch"

  # JSON output; hits[].score_parts breaks down every ranking factor:
  #   rrf         — fused relevance score (RRF/lexical/vector), neutral > 0
  #   activation  — ACT-R recency boost (post-clamp), neutral = 1.0
  #   feedback    — Beta-smoothed used/irrelevant ratio, neutral = 1.0
  #   quality     — frontmatter quality nudge (1-5 scale), neutral = 1.0
  #   supersede   — 0.2 penalty when superseded by a live memory, else 1.0
  #   final_score — product of all factors (== score at root level)
  comemory search "auth race" --json

  # Caller-supplied vector (BYO-vector, CSV form)
  comemory search "advisory lock" --vector 0.1,0.2,0.3,...
```

---

## comemory list

```
List memories with optional repo/kind filters

Usage: comemory list [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --repo <REPO>          Filter to memories whose `repo` matches exactly
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --kind <KIND>          Filter by kind (case-insensitive): decision|bug|convention|discovery|pattern|note
  -h, --help                 Print help

Examples:
  # All decisions in a single repo
  comemory list --repo myrepo --kind decision

  # Every memory across all repos, JSON
  comemory list --json

  # Filter by kind only
  comemory list --kind bug
```

---

## comemory delete

```
Soft-delete a memory by id (moves to `.trash/`)

Usage: comemory delete [OPTIONS] <ID>

Arguments:
  <ID>  8-hex memory id to delete

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Soft-delete by id (moves to memories/.trash/)
  comemory delete a1b2c3d4

  # JSON output for scripting
  comemory delete a1b2c3d4 --json
```

---

## comemory feedback

```
Record per-memory feedback (used vs irrelevant)

Usage: comemory feedback [OPTIONS] <QUERY_ID>

Arguments:
  <QUERY_ID>  Identifier of the originating search query (recorded for provenance)

Options:
      --json                     Emit machine-readable JSON instead of a human TTY view
      --used <USED>              Comma-separated memory ids that were used [default: ""]
      --data-dir <DATA_DIR>      Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --irrelevant <IRRELEVANT>  Comma-separated memory ids that were judged irrelevant [default: ""]
  -h, --help                     Print help

Examples:
  # Mark two hits as useful and one as irrelevant
  comemory feedback q-2026-05-17-001 --used a1b2c3d4,e5f6a7b8 --irrelevant 0011223344

  # Only-used feedback
  comemory feedback q-2026-05-17-002 --used a1b2c3d4

  # Only-irrelevant feedback
  comemory feedback q-2026-05-17-003 --irrelevant 0011223344
```

---

## comemory doctor

```
Report on the data directory and SQLite mirror health

Usage: comemory doctor [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Human-readable health report
  comemory doctor

  # JSON for monitoring or CI
  comemory doctor --json
```

---

## comemory index-code

```
Walk a repo, extract symbols, and upsert into the code index

Usage: comemory index-code [OPTIONS] --repo <REPO> --path <PATH>

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --repo <REPO>          Repo label stored alongside each symbol row
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --path <PATH>          Root of the working tree to walk. Must live inside a git repo so blob OIDs are available for the incremental skip path
      --extract              Emit JSONL on stdout instead of inserting rows. Suitable for piping into an external embedder + `comemory ingest-code`
  -h, --help                 Print help

Examples:
  # Index the current working directory with explicit repo label
  comemory index-code --repo myrepo --path .

  # Emit one JSONL row per symbol on stdout (skips DB writes)
  comemory index-code --repo myrepo --path ./src --extract
```

---

## comemory ingest-code

```
Read pre-embedded JSONL rows from stdin and ingest them into the code index (`code_symbols` + `code_fts` + `code_vec`)

Usage: comemory ingest-code [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Pipe pre-embedded JSONL from your embedder into the SQLite store
  comemory index-code --repo myrepo --path . --extract \
    | embed-snippets \
    | comemory ingest-code
```

---

## comemory ast

```
Run an ast-grep pattern against a single source file

Usage: comemory ast [OPTIONS] --lang <LANG> --file <FILE> <PATTERN>

Arguments:
  <PATTERN>  ast-grep pattern (`$VAR`, `$$$ARGS`, etc.)

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --lang <LANG>          Language tag: `rs`/`rust`, `ts`/`tsx`/`typescript`, `js`/`jsx`/`javascript`, `py`/`python`, `go`
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --file <FILE>          File to search
  -h, --help                 Print help

Examples:
  # Match every fn returning Result<_>
  comemory ast 'fn $NAME($$$ARGS) -> Result<$RET>' --lang rs --file src/db.rs

  # Find tokio::spawn call sites
  comemory ast 'tokio::spawn($$$)' --lang rs --file src/lib.rs --json

  # Hunt for `console.log` left in TypeScript
  comemory ast 'console.log($$$)' --lang ts --file src/index.ts
```

---

## comemory context

```
Headline lookup: code symbol + memories matching a key

Usage: comemory context [OPTIONS] <QUERY>

Arguments:
  <QUERY>  Free-form query — symbol name, file path fragment, or phrase

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --k <K>                Override the configured `retrieval.top_k` for this bundle. Must be >= 1
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --repo <REPO>          Optional repo filter forwarded to the router
      --vector <VECTOR>      Caller-supplied dense vector as a comma-separated float list. When provided together with `query`, both ANN and lexical branches run and their results are fused via RRF. Without a vector only the lexical FTS5 path runs
      --vector-stdin         Read a JSON `{ "embedding": [..] }` payload from stdin and use it as the dense vector for the context lookup. Mutually exclusive with reading the query from stdin
  -h, --help                 Print help

Examples:
  # Headline lookup for a symbol name, JSON envelope
  comemory context run_migration --json

  # Pin the bundle width to the top 3 hits
  comemory context "advisory lock" --k 3

  # ANN-assisted context with a caller-supplied vector
  comemory context "advisory lock" --vector 0.1,0.2,...
```

---

## comemory prune

```
Detect (and optionally soft-delete) stale memories

Usage: comemory prune [OPTIONS]

Options:
      --dry-run              Report candidates without applying any deletes
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Inspect candidates without mutating anything
  comemory prune --dry-run

  # Apply: soft-delete low-value memories (markdown -> memories/.trash/)
  # and clean up orphan edges + stale code symbols
  comemory prune

  # JSON output for CI/automation; Report fields:
  #   low_value_memories — ids matching ALL of: activation < COMEMORY_PRUNE_MIN_ACTIVATION
  #     (-2.0), Beta feedback <= COMEMORY_PRUNE_MIN_FEEDBACK (0.25), quality <=
  #     COMEMORY_PRUNE_BELOW_QUALITY (2), and zero incoming edges — OR superseded
  #     by a live memory with no access since the supersede edge was written.
  comemory prune --json
```

---

## comemory rebuild

```
Drop `comemory.db` and repopulate it from the markdown source of truth

Usage: comemory rebuild [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help
```

---

## comemory gc

```
Purge old entries from `memories/.trash/`

Usage: comemory gc [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Hard-delete .trash entries past the retention window
  comemory gc

  # JSON output for CI/automation
  comemory gc --json
```

---

## comemory install-hooks

```
Install git hooks that trigger `comemory index-code --incremental` on `post-commit`, `post-merge`, and `post-checkout`

Usage: comemory install-hooks [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --repo <REPO>          Repo root to install hooks into. Defaults to the current working directory [default: .]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --force                Overwrite existing hook files. Without this flag the command refuses to clobber a pre-existing `post-commit`/`post-merge`/`post-checkout` to avoid surprising users with hand-written hooks
  -h, --help                 Print help

Examples:
  # Install into the current repo
  comemory install-hooks

  # Install into a specific repo path
  comemory install-hooks --repo /path/to/repo

  # Overwrite any hand-written hooks
  comemory install-hooks --force
```

---

## comemory completions

```
Emit a shell completion script for `bash`, `zsh`, `fish`, `powershell`, or `elvish`

Usage: comemory completions [OPTIONS] <SHELL>

Arguments:
  <SHELL>  Shell to emit a completion script for [possible values: bash, elvish, fish, powershell, zsh]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # fish (autoloaded from this path)
  comemory completions fish > ~/.config/fish/completions/comemory.fish

  # zsh (homebrew site-functions path)
  comemory completions zsh > "$(brew --prefix)/share/zsh/site-functions/_comemory"

  # bash (homebrew bash-completion.d)
  comemory completions bash > "$(brew --prefix)/etc/bash_completion.d/comemory"

  # NOTE: scripts/dev-install.sh writes these automatically by default.
```

