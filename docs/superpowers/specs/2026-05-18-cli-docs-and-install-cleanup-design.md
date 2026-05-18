# CLI Documentation Enhancements + Install Cleanup (Design Spec)

**Status:** Draft for review
**Date:** 2026-05-18
**Author:** Falconiere R. Barbosa

---

## 1. Motivation

Two related friction points hit users on day one:

1. **Stale Python `qwick-memory` shim shadows the Rust binary.** The legacy
   uv-installed Python package leaves an executable at
   `~/.local/bin/qwick-memory` whose `qwick_memory` module is gone. Because
   `~/.local/bin` precedes `~/.cargo/bin` on a default `$PATH`, the broken
   shim wins and users see:

   ```
   ModuleNotFoundError: No module named 'qwick_memory'
   ```

2. **Per-subcommand help lacks runnable examples.** The clap-generated
   `--help` lists flags but stops there. The canonical examples live in
   `docs/cli-reference.md`, which the user has to discover separately.
   Shell completions are also missing, so daily UX (tab to discover
   subcommands and flags) is poor.

The first issue is an install ergonomics bug. The second is a
discoverability gap. Both are scoped tightly enough to bundle into one
spec.

This spec does **not** add man pages, an embedded `qwick-memory docs`
subcommand, or a web docs site — those are explicit non-goals.

---

## 2. Scope

Four deliverables in one PR:

1. **`qwick-memory completions <shell>` subcommand** — emits a completion
   script for `bash`, `zsh`, `fish`, `powershell`, or `elvish` on stdout.
2. **Inline `Examples:` block in every `--help`** — every subcommand's
   `--help` ends with usage examples via `#[command(after_help = …)]`.
3. **`docs/cli-reference.md` becomes a generated artifact** — sourced
   from the binary's own help output via `scripts/regen-cli-docs.sh`,
   guarded against drift by `scripts/cli-docs-check.sh` (added to the
   umbrella gate).
4. **`scripts/install.sh` auto-clean** — detects and removes competing
   `qwick-memory` installs (uv tool, brew formula) before the cargo
   install. Default-on; opt out with `--no-clean`. Warns when a PATH
   entry shadows the freshly installed binary.

---

## 3. Non-Goals

- Man pages (`clap_mangen`). `--help` plus shell completions cover the
  use case for now.
- An embedded `qwick-memory docs` subcommand that prints architecture
  excerpts. `docs/cli-reference.md` is already linked from the README.
- A web documentation site.
- Any change to subcommand *behavior*. This is help/install plumbing
  only.
- `cargo uninstall` of a prior Rust install. `cargo install --force`
  already replaces it idempotently.
- Removing arbitrary `qwick-memory` files via `rm`. Only package
  managers (uv, brew) are used for cleanup.

---

## 4. User-facing surface

### 4.1 `qwick-memory completions`

```
Generate a shell completion script

Usage: qwick-memory completions [OPTIONS] <SHELL>

Arguments:
  <SHELL>  Shell to emit a completion script for [possible values: bash, zsh, fish, powershell, elvish]

Examples:
  qwick-memory completions fish > ~/.config/fish/completions/qwick-memory.fish
  qwick-memory completions zsh  > "${fpath[1]}/_qwick-memory"
  qwick-memory completions bash > /usr/local/etc/bash_completion.d/qwick-memory
```

The command writes to stdout only. Users redirect to the appropriate
location for their shell. No filesystem side effects.

### 4.2 `Examples:` block in every subcommand's `--help`

Every existing subcommand (`save`, `search`, `list`, `delete`, …) gains
a trailing examples block in its `--help`. The examples are the same
ones already in `docs/cli-reference.md`. Example:

```
$ qwick-memory save --help
Save a memory (body via arg, `-`, or stdin)

Usage: qwick-memory save [OPTIONS] [BODY]

Arguments:
  [BODY]  Memory body. Use `-` (or omit) to read from stdin

Options:
      --kind <KIND>          Memory kind: decision|bug|convention|discovery|pattern|note [default: note]
      --repo <REPO>          Repo name attached to the memory (free-form string) [default: ""]
      --tags <TAGS>          Comma-separated tag list (e.g. `database,postgres`) [default: ""]
      --author <AUTHOR>      Author identifier [default: ""]
      --quality <QUALITY>    Quality rating 1..=5 [default: 3]
  -h, --help                 Print help

Examples:
  qwick-memory save "Use Postgres for analytics" --kind decision --repo myrepo --tags db,postgres --quality 4
  echo "Race in run_migration when run twice in <1s" | qwick-memory save - --kind bug --repo myrepo
```

### 4.3 `scripts/install.sh`

New flag `--no-clean` opts out of competing-install removal. The
existing `--with-tools` flag is unchanged and remains orthogonal.

```
scripts/install.sh                # build + install + auto-clean
scripts/install.sh --no-clean     # build + install, leave competing installs alone
scripts/install.sh --with-tools   # add sccache + hyperfine
```

Auto-clean output sample:

```
[install] detected uv tool: qwick-memory — uninstalling
[install] detected brew formula: qwick-memory — uninstalling
[install] building release-quick binary
[install] installing into cargo bin (release-quick profile)
[install] installed /Users/me/.cargo/bin/qwick-memory (qwick-memory 1.1.0)
[install] warning: /Users/me/.local/bin/qwick-memory still on PATH ahead of /Users/me/.cargo/bin (rehash your shell)
```

---

## 5. Architecture

### 5.1 Single source of truth for help/examples

Examples must not be authored in two places. The chosen direction:

- **Examples live in Rust source** as `const _EXAMPLES: &str = "…"`
  per subcommand module.
- `#[command(after_help = SAVE_EXAMPLES)]` on each `Args` struct (or on
  the `Cmd` variant for unit-only commands like `Doctor` and `Gc`).
- `docs/cli-reference.md` becomes a **generated artifact**, produced by
  `scripts/regen-cli-docs.sh` from `qwick-memory <cmd> --help` output.
- `scripts/cli-docs-check.sh` regenerates into a tmpfile and diffs
  against the checked-in markdown; non-zero exit on drift. Added to
  `scripts/check-all.sh` so the umbrella gate enforces consistency.

This satisfies binding rule #1 (no duplication). Rebuilding the binary
and re-running `scripts/regen-cli-docs.sh` is the single workflow for
updating the public help and the markdown together.

### 5.2 Module layout

New module: `src/cli/completions.rs`.

```
src/cli/
├── mod.rs                # add Cmd::Completions variant + dispatch
├── completions.rs        # NEW: Args { shell: clap_complete::Shell }, run
├── save.rs               # add const SAVE_EXAMPLES + #[command(after_help)]
├── search.rs             # add const SEARCH_EXAMPLES + #[command(after_help)]
├── … (every other subcommand follows the same pattern)
```

All files stay under the 500-line cap (each adds ≤10 lines). Each
`tests/cli/<cmd>.rs` similarly gains a `--help` assertion submodule.

### 5.3 `clap_complete` integration

`src/cli/completions.rs` is intentionally small:

```rust
//! `qwick-memory completions <shell>` — emit a completion script on stdout.

use clap::{Args as ClapArgs, CommandFactory};
use clap_complete::{generate, Shell};
use std::io;
use std::path::PathBuf;

use crate::cli::Cli;
use crate::prelude::*;

const COMPLETIONS_EXAMPLES: &str = "\
Examples:
  qwick-memory completions fish > ~/.config/fish/completions/qwick-memory.fish
  qwick-memory completions zsh  > \"${fpath[1]}/_qwick-memory\"
  qwick-memory completions bash > /usr/local/etc/bash_completion.d/qwick-memory";

#[derive(ClapArgs, Debug)]
#[command(after_help = COMPLETIONS_EXAMPLES)]
pub struct Args {
    /// Shell to emit a completion script for.
    pub shell: Shell,
}

pub async fn run(a: Args, _json: bool, _data_dir: Option<PathBuf>) -> Result<()> {
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    let mut out = io::stdout().lock();
    generate(a.shell, &mut cmd, bin_name, &mut out);
    Ok(())
}
```

The `--json` and `--data-dir` globals are accepted but ignored; the
output is shell script text by definition.

### 5.4 `Cargo.toml`

Add one dependency:

```toml
clap_complete = "4"
```

Pinned to the same major version as `clap = "4"`.

### 5.5 `scripts/install.sh` cleanup logic

Added before the existing build step. Pseudocode (real implementation
lives in `scripts/install.sh` and uses the existing `lib/common.sh`
loggers):

```bash
CLEAN=1
for arg in "$@"; do
  case "$arg" in
    --no-clean)   CLEAN=0 ;;
    --with-tools) WITH_TOOLS=1 ;;
  esac
done

if [[ "$CLEAN" -eq 1 ]]; then
  if command -v uv >/dev/null 2>&1 \
     && uv tool list 2>/dev/null | grep -q '^qwick-memory'; then
    log_info "$STEP" "detected uv tool: qwick-memory — uninstalling"
    uv tool uninstall qwick-memory >/dev/null
  fi
  if command -v brew >/dev/null 2>&1 \
     && brew list --formula 2>/dev/null | grep -qx 'qwick-memory'; then
    log_info "$STEP" "detected brew formula: qwick-memory — uninstalling"
    brew uninstall qwick-memory >/dev/null
  fi
fi
```

After the install completes, walk `$PATH` from left to right. The
first `qwick-memory` hit that is **not** equal to `$BIN_PATH` is logged
as a warning so the user can investigate (e.g. shell rehash needed,
non-uv/brew install lingering).

### 5.6 Out-of-scope cleanup paths

- Pipx, asdf, nix profiles: rare for this binary and not detected.
  Users on those can `--no-clean` and remove manually.
- Bare `~/bin/qwick-memory` copies: warned via the PATH shadow check
  but never deleted.

---

## 6. Testing

| Surface | Test |
|---|---|
| `qwick-memory completions <shell>` for each of 5 shells | `tests/cli/completions/<shell>.rs` runs `assert_cmd`, asserts exit 0, non-empty stdout, and that `qwick-memory` appears at least once in the output |
| Inline `Examples:` block | `tests/cli/help_examples.rs` parametrizes over every subcommand and asserts that `<cmd> --help` contains the literal token `Examples:` and at least one ` qwick-memory ` token below it |
| Help/docs drift | `scripts/cli-docs-check.sh` regenerates into `target/cli-reference.generated.md` and diffs against `docs/cli-reference.md`. Runs in `scripts/check-all.sh`. Failure prints the diff for easy local fix-up. |
| `scripts/install.sh` | No automated test (existing repo policy for shell scripts). Manual verification step in PR description: run on a machine with stale uv shim and confirm shim is gone afterwards |

Test placement follows binding rule #5: every new `src/cli/<cmd>.rs`
gets a 1:1 mirror in `tests/cli/<cmd>.rs`, with a thin shim that
declares submodules in `tests/cli/<cmd>/`.

---

## 7. Binding Rules Compliance

1. **No duplication** — examples are authored once (Rust source);
   markdown is generated. Drift guarded by `cli-docs-check.sh`.
2. **Modularity** — `completions.rs` is its own module with a single
   purpose. Examples live next to the subcommand they document.
3. **≤500 lines per file** — additions are tens of lines per file.
   `completions.rs` is ~30 lines including doc comments.
4. **Zero errors / zero warnings** — uses `tracing::warn!` for the
   install PATH-shadow notice in any future Rust port; current
   `install.sh` already uses the existing logger helpers. No new
   `unwrap` / `expect` / `println!` introduced.
5. **Tests in `tests/` mirroring `src/`** — every new source file gets
   a mirror test file.

`scripts/dup-check.sh` and `scripts/module-size-check.sh` continue to
pass.

---

## 8. Rollout

This is a single PR. No migrations. No breaking changes.

After merge, the existing v1 user who hit the Python traceback can:

```bash
git pull
scripts/install.sh        # auto-removes the uv shim, installs Rust binary
hash -r                   # or `rehash` in fish
qwick-memory --help       # now succeeds, with examples in every subcommand's help
qwick-memory completions fish > ~/.config/fish/completions/qwick-memory.fish
```

---

## 9. Open Questions

None as of this draft. Approve to proceed to the implementation plan.
