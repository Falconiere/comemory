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
  doctor         Report on the data directory and memory count
  index          Memory-layer index maintenance (re-embed missing rows). Run `comemory index --help` for the available flags
  index-code     Walk a repo, extract symbols, and upsert into the code index
  ingest-code    Read pre-embedded JSONL rows from stdin and ingest them into the code index (`code_symbols` + `code_fts` + `code_vec`)
  symbol         Semantic search over the code index for a symbol name
  memory-for     List memories that reference a qualified symbol or file path
  ast            Run an ast-grep pattern against a single source file
  context        Headline lookup: code symbol + memories matching a key
  walk           Walk a graph edge from a memory id (currently `--edge supersedes`)
  completions    Emit a shell completion script for `bash`, `zsh`, `fish`, `powershell`, or `elvish`
  conflicts      List memories that conflict with the given memory id
  supersedes     Record that one memory supersedes another in the kuzu graph
  prune          Detect (and optionally soft-delete) stale memories
  gc             Purge old entries from `memories/.trash/`
  install-hooks  Install git hooks that trigger `comemory index-code --incremental` on `post-commit`, `post-merge`, and `post-checkout`
  graph          Property-graph tooling. Run `comemory graph --help`
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
      --json                 Emit machine-readable JSON instead of a human TTY view
      --kind <KIND>          Memory kind: decision|bug|convention|discovery|pattern|note [default: note] [possible values: decision, bug, convention, discovery, pattern, note]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --repo <REPO>          Repo name attached to the memory (free-form string) [default: ""]
      --tags <TAGS>          Comma-separated tag list (e.g. `database,postgres`) [default: ""]
      --author <AUTHOR>      Author identifier. Defaults to empty so callers may omit [default: ""]
      --quality <QUALITY>    Quality rating 1..=5. Defaults to 3 [default: 3]
      --vector <VECTOR>      Caller-supplied dense vector as a comma-separated float list. Length must equal the configured memory vector dim or the save fails with `vector dim mismatch`
      --vector-stdin         Read a JSON `{ "embedding": [..] }` payload from stdin and use it as the dense vector for the saved memory. Mutually exclusive with body being read from stdin (the body must be supplied as a positional arg when `--vector-stdin` is set)
  -h, --help                 Print help

Examples:
  # Save a decision with tags and elevated quality
  comemory save "Use Postgres for analytics" --kind decision --repo myrepo --tags db,postgres --quality 4

  # Pipe a bug report body from another command
  echo "Race in run_migration when run twice in <1s" | comemory save - --kind bug --repo myrepo

  # Save with a caller-supplied embedding (BYO-vector)
  echo '{"embedding":[0.1,0.2,...]}' | comemory save "...body..." --vector-stdin

  # Minimal note (kind defaults to `note`, no repo/tags)
  comemory save "Remember: cargo nextest serializes the embedder group"
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
  # Natural-language query, top 12 hits (default)
  comemory search "postgres migration race"

  # JSON envelope for piping into other tools
  comemory search "advisory lock" --json

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
  <ID>  12-hex memory id to delete

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
Report on the data directory and memory count

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

## comemory index

```
Memory-layer index maintenance (re-embed missing rows). Run `comemory index --help` for the available flags

Usage: comemory index [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --rebuild              Re-embed any markdown memory whose id is missing from the dense `memory_chunks` table
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --quiet                Suppress the human-readable summary line. JSON output is still emitted when `--json` is set
  -h, --help                 Print help

Examples:
  # Re-embed every markdown memory missing from the dense index
  comemory index --rebuild

  # JSON summary for monitoring / CI
  comemory index --rebuild --json

  # Quiet rebuild (suppresses the human summary; JSON still respected)
  comemory index --rebuild --quiet
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

## comemory symbol

```
Semantic search over the code index for a symbol name

Usage: comemory symbol [OPTIONS] <NAME>

Arguments:
  <NAME>  Free-form symbol name (or descriptor) to search for

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --limit <LIMIT>        Maximum number of hits to return (default 5) [default: 5]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Exact function-name hit
  comemory symbol run_migration
```

---

## comemory memory-for

```
List memories that reference a qualified symbol or file path

Usage: comemory memory-for [OPTIONS] <QUALIFIED>

Arguments:
  <QUALIFIED>  Qualified symbol (`<repo>:<path>:<symbol>`) or file path (`<repo>:<path>`) to look up

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Memories that reference a specific function
  comemory memory-for myrepo:src/db.rs:run_migration

  # Memories that reference a whole file
  comemory memory-for myrepo:src/db.rs

  # JSON for tool chaining
  comemory memory-for myrepo:src/db.rs --json
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
  -h, --help                 Print help

Examples:
  # Headline lookup for a symbol name, JSON envelope
  comemory context run_migration --json

  # Pin the bundle width to the top 3 hits
  comemory context "advisory lock" --k 3
```

---

## comemory walk

```
Walk a graph edge from a memory id (currently `--edge supersedes`)

Usage: comemory walk [OPTIONS] --from <FROM>

Options:
      --from <FROM>          Memory id to start walking from
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --edge <EDGE>          Edge kind to traverse. Currently only `supersedes` is supported [default: supersedes]
      --depth <DEPTH>        Maximum hop depth. Clamped to at least 1 by the underlying query [default: 5]
  -h, --help                 Print help

Examples:
  # Trace a supersedes chain up to 5 hops (JSON)
  comemory walk --from a1b2c3d4 --edge supersedes --depth 5 --json

  # Single-hop walk (default edge = supersedes)
  comemory walk --from a1b2c3d4 --depth 1
```

---

## comemory conflicts

```
List memories that conflict with the given memory id

Usage: comemory conflicts [OPTIONS] <ID>

Arguments:
  <ID>  Memory id whose outgoing `:ConflictsWith` edges should be listed

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # List ConflictsWith neighbors of a memory
  comemory conflicts a1b2c3d4

  # JSON output
  comemory conflicts a1b2c3d4 --json
```

---

## comemory supersedes

```
Record that one memory supersedes another in the kuzu graph

Usage: comemory supersedes [OPTIONS] <NEW_ID> <OLD_ID>

Arguments:
  <NEW_ID>  Memory id of the **new** decision (the one that supersedes)
  <OLD_ID>  Memory id of the **old** decision (the one being superseded)

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Mark e5f6a7b8 as superseding the older decision a1b2c3d4
  comemory supersedes e5f6a7b8 a1b2c3d4
```

---

## comemory prune

```
Detect (and optionally soft-delete) stale memories

Usage: comemory prune [OPTIONS]

Options:
      --json                           Emit machine-readable JSON instead of a human TTY view
      --orphans                        Detect orphan entries in `memories/.trash/`
      --data-dir <DATA_DIR>            Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --low-value                      Detect low-value memories (quality + unused + age gates)
      --below-quality <BELOW_QUALITY>  Strict upper bound on quality for low-value matches [default: 2]
      --unused-since <UNUSED_SINCE>    Minimum age in days (since `created`) for low-value matches [default: 180]
      --apply                          Perform soft-deletes instead of a dry-run
  -h, --help                           Print help

Examples:
  # Dry-run orphan detection (no deletes)
  comemory prune --orphans

  # Actually move orphans to memories/.trash/
  comemory prune --orphans --apply

  # Aggressive low-value sweep
  comemory prune --low-value --below-quality 2 --unused-since 180 --apply
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

  # NOTE: scripts/install.sh writes these automatically by default.
```

---

## comemory graph

```
Property-graph tooling. Run `comemory graph --help`

Usage: comemory graph [OPTIONS] <COMMAND>

Commands:
  serve  Spin up the local HTTP viewer for the property graph
  help   Print this message or the help of the given subcommand(s)

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help
```

---

## comemory graph serve

```
Spin up the local HTTP viewer for the property graph

Usage: comemory graph serve [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --port <PORT>          Override the bind port. `0` lets the kernel pick a free port [default: 0]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --no-open              Skip auto-opening the URL in the system browser
      --host <HOST>          Bind address. Loopback by default [default: 127.0.0.1]
      --bind-public          Required when `--host` is non-loopback. Acknowledges the network exposure: the viewer is read-only but unauthenticated
  -h, --help                 Print help

Examples:
  # Open the viewer in the default browser
  comemory graph serve

  # Headless / over SSH
  comemory graph serve --no-open

  # Pin a port
  comemory graph serve --port 7878
```

