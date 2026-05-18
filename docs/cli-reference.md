# CLI reference

This page is **generated** by `scripts/regen-cli-docs.sh`. Do not edit by
hand — re-run the script and commit the result. Drift is enforced by
`scripts/cli-docs-check.sh` in the umbrella gate.

For the design rationale behind each command, see the
[design spec](superpowers/specs/2026-05-17-qwick-rust-agentic-rag-design.md).

## Global options

Every subcommand inherits two global flags:

| Flag | Description |
|---|---|
| `--json` | Emit machine-readable JSON instead of a human TTY view. |
| `--data-dir <DATA_DIR>` | Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable. |

Exit codes follow `sysexits.h`: `0` success, non-zero for usage / I/O /
data errors.

## Top-level help

```
Agentic dev memory + code-aware semantic search

Usage: qwick-memory [OPTIONS] <COMMAND>

Commands:
  save           Save a memory (body via arg, `-`, or stdin)
  search         Search the memory index by natural-language query
  list           List memories with optional repo/kind filters
  delete         Soft-delete a memory by id (moves to `.trash/`)
  feedback       Record per-memory feedback (used vs irrelevant)
  doctor         Report on the data directory and memory count
  index-code     Walk a repo, extract symbols, and upsert into the code index
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
  install-hooks  Install git hooks that trigger `qwick-memory index-code --incremental` on `post-commit`, `post-merge`, and `post-checkout`
  help           Print this message or the help of the given subcommand(s)

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
  -h, --help                 Print help
  -V, --version              Print version
```

---

## qwick-memory save

```
Save a memory (body via arg, `-`, or stdin)

Usage: qwick-memory save [OPTIONS] [BODY]

Arguments:
  [BODY]  Memory body. Use `-` (or omit) to read from stdin

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --kind <KIND>          Memory kind: decision|bug|convention|discovery|pattern|note [default: note] [possible values: decision, bug, convention, discovery, pattern, note]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
      --repo <REPO>          Repo name attached to the memory (free-form string) [default: ""]
      --tags <TAGS>          Comma-separated tag list (e.g. `database,postgres`) [default: ""]
      --author <AUTHOR>      Author identifier. Defaults to empty so callers may omit [default: ""]
      --quality <QUALITY>    Quality rating 1..=5. Defaults to 3 [default: 3]
  -h, --help                 Print help

Examples:
  qwick-memory save "Use Postgres for analytics" --kind decision --repo myrepo --tags db,postgres --quality 4
  echo "Race in run_migration when run twice in <1s" | qwick-memory save - --kind bug --repo myrepo
```

---

## qwick-memory search

```
Search the memory index by natural-language query

Usage: qwick-memory search [OPTIONS] <QUERY>

Arguments:
  <QUERY>  Natural-language query string

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --limit <LIMIT>        Maximum number of hits to return (default 12) [default: 12]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  qwick-memory search "postgres migration race"
  qwick-memory search "what database do we use" --limit 5 --json
```

---

## qwick-memory list

```
List memories with optional repo/kind filters

Usage: qwick-memory list [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --repo <REPO>          Filter to memories whose `repo` matches exactly
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
      --kind <KIND>          Filter by kind (case-insensitive): decision|bug|convention|discovery|pattern|note
  -h, --help                 Print help

Examples:
  qwick-memory list --repo myrepo --kind decision
  qwick-memory list --json
```

---

## qwick-memory delete

```
Soft-delete a memory by id (moves to `.trash/`)

Usage: qwick-memory delete [OPTIONS] <ID>

Arguments:
  <ID>  12-hex memory id to delete

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  qwick-memory delete a1b2c3d4
  qwick-memory delete a1b2c3d4 --json
```

---

## qwick-memory feedback

```
Record per-memory feedback (used vs irrelevant)

Usage: qwick-memory feedback [OPTIONS] <QUERY_ID>

Arguments:
  <QUERY_ID>  Identifier of the originating search query (recorded for provenance)

Options:
      --json                     Emit machine-readable JSON instead of a human TTY view
      --used <USED>              Comma-separated memory ids that were used [default: ""]
      --data-dir <DATA_DIR>      Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
      --irrelevant <IRRELEVANT>  Comma-separated memory ids that were judged irrelevant [default: ""]
  -h, --help                     Print help

Examples:
  qwick-memory feedback q-2026-05-17-001 --used a1b2c3d4,e5f6a7b8 --irrelevant 0011223344
```

---

## qwick-memory doctor

```
Report on the data directory and memory count

Usage: qwick-memory doctor [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  qwick-memory doctor
  qwick-memory doctor --json
```

---

## qwick-memory index-code

```
Walk a repo, extract symbols, and upsert into the code index

Usage: qwick-memory index-code [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --root <ROOT>          Repo root to walk. Defaults to the current working directory [default: .]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
      --repo <REPO>          Repo label stored in the `qualified` key. Auto-detected from `root` basename when empty [default: ""]
      --incremental          Skip rows whose `ast_hash` is unchanged. Reserved for Task 19; accepted but currently a no-op
      --quiet                Suppress the human-readable summary line. JSON output is still emitted when `--json` is set
  -h, --help                 Print help

Examples:
  qwick-memory index-code --root . --repo myrepo
  qwick-memory index-code --root /path/to/repo --incremental --quiet
```

---

## qwick-memory symbol

```
Semantic search over the code index for a symbol name

Usage: qwick-memory symbol [OPTIONS] <NAME>

Arguments:
  <NAME>  Free-form symbol name (or descriptor) to search for

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --limit <LIMIT>        Maximum number of hits to return (default 5) [default: 5]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  qwick-memory symbol run_migration
  qwick-memory symbol "parse frontmatter yaml" --limit 10 --json
```

---

## qwick-memory memory-for

```
List memories that reference a qualified symbol or file path

Usage: qwick-memory memory-for [OPTIONS] <QUALIFIED>

Arguments:
  <QUALIFIED>  Qualified symbol (`<repo>:<path>:<symbol>`) or file path (`<repo>:<path>`) to look up

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  qwick-memory memory-for myrepo:src/db.rs:run_migration
  qwick-memory memory-for myrepo:src/db.rs
```

---

## qwick-memory ast

```
Run an ast-grep pattern against a single source file

Usage: qwick-memory ast [OPTIONS] --lang <LANG> --file <FILE> <PATTERN>

Arguments:
  <PATTERN>  ast-grep pattern (`$VAR`, `$$$ARGS`, etc.)

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --lang <LANG>          Language tag: `rs`/`rust`, `ts`/`tsx`, `js`/`jsx`, `py`
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
      --file <FILE>          File to search
  -h, --help                 Print help

Examples:
  qwick-memory ast 'fn $NAME($$$ARGS) -> Result<$RET>' --lang rs --file src/db.rs
  qwick-memory ast 'tokio::spawn($$$)' --lang rs --file src/lib.rs --json
```

---

## qwick-memory context

```
Headline lookup: code symbol + memories matching a key

Usage: qwick-memory context [OPTIONS] <KEY>

Arguments:
  <KEY>  Free-form key — symbol name, file path fragment, or natural-language phrase. Embedded against both the code index and the memory index

Options:
      --depth <DEPTH>        Graph-walk depth. Reserved for Task 17 (Supersedes / ConflictsWith walks); accepted now to keep the CLI shape stable [default: 1]
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  qwick-memory context run_migration --json
  qwick-memory context "postgres migration race" --depth 2
```

---

## qwick-memory walk

```
Walk a graph edge from a memory id (currently `--edge supersedes`)

Usage: qwick-memory walk [OPTIONS] --from <FROM>

Options:
      --from <FROM>          Memory id to start walking from
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
      --edge <EDGE>          Edge kind to traverse. Currently only `supersedes` is supported [default: supersedes]
      --depth <DEPTH>        Maximum hop depth. Clamped to at least 1 by the underlying query [default: 5]
  -h, --help                 Print help

Examples:
  qwick-memory walk --from a1b2c3d4 --edge supersedes --depth 5 --json
```

---

## qwick-memory conflicts

```
List memories that conflict with the given memory id

Usage: qwick-memory conflicts [OPTIONS] <ID>

Arguments:
  <ID>  Memory id whose outgoing `:ConflictsWith` edges should be listed

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  qwick-memory conflicts a1b2c3d4
  qwick-memory conflicts a1b2c3d4 --json
```

---

## qwick-memory supersedes

```
Record that one memory supersedes another in the kuzu graph

Usage: qwick-memory supersedes [OPTIONS] <NEW_ID> <OLD_ID>

Arguments:
  <NEW_ID>  Memory id of the **new** decision (the one that supersedes)
  <OLD_ID>  Memory id of the **old** decision (the one being superseded)

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  qwick-memory supersedes e5f6a7b8 a1b2c3d4
```

---

## qwick-memory prune

```
Detect (and optionally soft-delete) stale memories

Usage: qwick-memory prune [OPTIONS]

Options:
      --json                           Emit machine-readable JSON instead of a human TTY view
      --orphans                        Detect orphan entries in `memories/.trash/`
      --data-dir <DATA_DIR>            Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
      --low-value                      Detect low-value memories (quality + unused + age gates)
      --below-quality <BELOW_QUALITY>  Strict upper bound on quality for low-value matches [default: 2]
      --unused-since <UNUSED_SINCE>    Minimum age in days (since `created`) for low-value matches [default: 180]
      --apply                          Perform soft-deletes instead of a dry-run
  -h, --help                           Print help

Examples:
  qwick-memory prune --orphans
  qwick-memory prune --orphans --apply
  qwick-memory prune --low-value --below-quality 2 --unused-since 180 --apply
```

---

## qwick-memory gc

```
Purge old entries from `memories/.trash/`

Usage: qwick-memory gc [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  qwick-memory gc
  qwick-memory gc --json
```

---

## qwick-memory install-hooks

```
Install git hooks that trigger `qwick-memory index-code --incremental` on `post-commit`, `post-merge`, and `post-checkout`

Usage: qwick-memory install-hooks [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --repo <REPO>          Repo root to install hooks into. Defaults to the current working directory [default: .]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
      --force                Overwrite existing hook files. Without this flag the command refuses to clobber a pre-existing `post-commit`/`post-merge`/`post-checkout` to avoid surprising users with hand-written hooks
  -h, --help                 Print help

Examples:
  qwick-memory install-hooks --repo .
  qwick-memory install-hooks --repo /path/to/repo --force
```

---

## qwick-memory completions

```
Emit a shell completion script for `bash`, `zsh`, `fish`, `powershell`, or `elvish`

Usage: qwick-memory completions [OPTIONS] <SHELL>

Arguments:
  <SHELL>  Shell to emit a completion script for [possible values: bash, elvish, fish, powershell, zsh]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable [env: QWICK_MEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  qwick-memory completions fish > ~/.config/fish/completions/qwick-memory.fish
  qwick-memory completions zsh  > "${fpath[1]}/_qwick-memory"
  qwick-memory completions bash > /usr/local/etc/bash_completion.d/qwick-memory
```

