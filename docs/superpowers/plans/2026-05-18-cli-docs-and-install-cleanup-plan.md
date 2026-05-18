# CLI Documentation Enhancements + Install Cleanup — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `qwick-memory completions <shell>`, inline `Examples:` blocks in every subcommand's `--help`, a generated `docs/cli-reference.md` guarded by drift check, and an auto-cleaning `scripts/install.sh` that removes competing uv/brew installs.

**Architecture:** Examples live in Rust source as `const _EXAMPLES: &str` per subcommand module and are exposed via `#[command(after_help = …)]`. Markdown is generated from binary help output. Drift is enforced by `scripts/cli-docs-check.sh` in `scripts/check-all.sh`. Install cleanup runs through package managers only (uv, brew); never `rm`.

**Tech Stack:** Rust, clap 4 (`derive`), `clap_complete = "4"`, `assert_cmd`, `tempfile`, bash (scripts), `diff(1)`.

**Spec:** `docs/superpowers/specs/2026-05-18-cli-docs-and-install-cleanup-design.md`

---

## File Structure

**Create:**
- `src/cli/completions.rs` — new subcommand module, ~40 lines, single purpose (emit shell completion script).
- `tests/cli_completions.rs` — `assert_cmd` integration tests, one per shell.
- `tests/cli_help_examples.rs` — parametric `--help` assertion across every subcommand.
- `scripts/regen-cli-docs.sh` — runs the binary's `--help` for each subcommand and emits `docs/cli-reference.md`.
- `scripts/cli-docs-check.sh` — regenerates into a tmpfile and diffs against the checked-in markdown.

**Modify:**
- `Cargo.toml` — add `clap_complete = "4"` dependency.
- `src/cli/mod.rs` — declare `pub mod completions;`, add `Cmd::Completions(completions::Args)` variant, dispatch in `run`.
- `src/cli/save.rs`, `search.rs`, `list.rs`, `delete.rs`, `feedback.rs`, `doctor.rs`, `index_code.rs`, `symbol.rs`, `memory_for.rs`, `ast.rs`, `context.rs`, `walk.rs`, `conflicts.rs`, `supersedes.rs`, `prune.rs`, `gc.rs`, `install_hooks.rs` — add `const _EXAMPLES: &str` and `#[command(after_help = _EXAMPLES)]`.
- `docs/cli-reference.md` — regenerated artifact, now sourced from binary help.
- `scripts/check-all.sh` — append `cli-docs-check.sh` invocation.
- `scripts/install.sh` — parse `--no-clean`, detect/uninstall uv tool + brew formula, warn on PATH shadow after install.

**Each modified file stays well under the 500-line cap.**

---

## Task 1: Add `clap_complete` dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Read current `[dependencies]` block to confirm clap version**

Run: `grep -nE '^clap' Cargo.toml`
Expected: `clap = { version = "4", features = ["derive", "env"] }`

- [ ] **Step 2: Add `clap_complete` next to `clap`**

In `Cargo.toml`, in `[dependencies]`, immediately after the `clap = …` line, add:

```toml
clap_complete = "4"
```

- [ ] **Step 3: Verify it resolves**

Run: `cargo check --all-targets --all-features`
Expected: exit 0, no `clap_complete` errors. `Cargo.lock` updated.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: add clap_complete dep for shell completions"
```

---

## Task 2: Write failing test for `qwick-memory completions <shell>`

**Files:**
- Create: `tests/cli_completions.rs`

- [ ] **Step 1: Write the failing test file**

Create `tests/cli_completions.rs` with:

```rust
//! Integration tests for `qwick-memory completions <shell>`.

use assert_cmd::Command;

fn run_completions(shell: &str) -> assert_cmd::assert::Assert {
    Command::cargo_bin("qwick-memory")
        .expect("cargo_bin qwick-memory")
        .args(["completions", shell])
        .assert()
}

#[test]
fn fish_emits_completion_script() {
    let out = run_completions("fish").success().get_output().stdout.clone();
    let body = String::from_utf8(out).expect("fish completions are utf-8");
    assert!(!body.trim().is_empty(), "fish completions stdout is empty");
    assert!(
        body.contains("qwick-memory"),
        "fish completions missing binary name"
    );
}

#[test]
fn bash_emits_completion_script() {
    let out = run_completions("bash").success().get_output().stdout.clone();
    let body = String::from_utf8(out).expect("bash completions are utf-8");
    assert!(!body.trim().is_empty(), "bash completions stdout is empty");
    assert!(
        body.contains("qwick-memory"),
        "bash completions missing binary name"
    );
}

#[test]
fn zsh_emits_completion_script() {
    let out = run_completions("zsh").success().get_output().stdout.clone();
    let body = String::from_utf8(out).expect("zsh completions are utf-8");
    assert!(!body.trim().is_empty(), "zsh completions stdout is empty");
    assert!(
        body.contains("qwick-memory"),
        "zsh completions missing binary name"
    );
}

#[test]
fn powershell_emits_completion_script() {
    let out = run_completions("powershell").success().get_output().stdout.clone();
    let body = String::from_utf8(out).expect("powershell completions are utf-8");
    assert!(!body.trim().is_empty(), "powershell completions stdout is empty");
    assert!(
        body.contains("qwick-memory"),
        "powershell completions missing binary name"
    );
}

#[test]
fn elvish_emits_completion_script() {
    let out = run_completions("elvish").success().get_output().stdout.clone();
    let body = String::from_utf8(out).expect("elvish completions are utf-8");
    assert!(!body.trim().is_empty(), "elvish completions stdout is empty");
    assert!(
        body.contains("qwick-memory"),
        "elvish completions missing binary name"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --test cli_completions`
Expected: 5 FAIL with "error: unrecognized subcommand 'completions'".

---

## Task 3: Create `src/cli/completions.rs`

**Files:**
- Create: `src/cli/completions.rs`

- [ ] **Step 1: Write the module**

Create `src/cli/completions.rs` with:

```rust
//! `qwick-memory completions <shell>` — emit a shell completion script on stdout.
//!
//! Wraps `clap_complete::generate` against the top-level `Cli` so completions
//! always reflect the current subcommand surface. The `--json` and
//! `--data-dir` globals are accepted but ignored: this subcommand only
//! produces shell script text.

use std::io;
use std::path::PathBuf;

use clap::{Args as ClapArgs, CommandFactory};
use clap_complete::{generate, Shell};

use crate::cli::Cli;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  qwick-memory completions fish > ~/.config/fish/completions/qwick-memory.fish
  qwick-memory completions zsh  > \"${fpath[1]}/_qwick-memory\"
  qwick-memory completions bash > /usr/local/etc/bash_completion.d/qwick-memory";

/// Arguments for `qwick-memory completions`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Shell to emit a completion script for.
    pub shell: Shell,
}

/// Emit the completion script for `a.shell` on stdout.
pub async fn run(a: Args, _json: bool, _data_dir: Option<PathBuf>) -> Result<()> {
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    let mut out = io::stdout().lock();
    generate(a.shell, &mut cmd, bin_name, &mut out);
    Ok(())
}
```

---

## Task 4: Wire `completions` into the CLI dispatcher

**Files:**
- Modify: `src/cli/mod.rs`

- [ ] **Step 1: Declare the module**

In `src/cli/mod.rs`, in the alphabetised `pub mod …;` block, add (after `pub mod conflicts;`):

```rust
pub mod completions;
```

- [ ] **Step 2: Add the enum variant**

In the `Cmd` enum in `src/cli/mod.rs`, add (alphabetised between `Conflicts` and `Context`):

```rust
    /// Emit a shell completion script for `bash`, `zsh`, `fish`, `powershell`, or `elvish`.
    Completions(completions::Args),
```

- [ ] **Step 3: Add the dispatch arm**

In `pub async fn run(cli: Cli)` in `src/cli/mod.rs`, in the `match cli.cmd` block, add (matching the enum order):

```rust
        Cmd::Completions(a) => completions::run(a, cli.json, cli.data_dir).await,
```

- [ ] **Step 4: Run the completion tests to verify they pass**

Run: `cargo nextest run --test cli_completions`
Expected: 5 PASS.

- [ ] **Step 5: Run the full umbrella gate so format/lint/tests stay green**

Run: `bash scripts/check-all.sh`
Expected: exit 0.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/cli/completions.rs src/cli/mod.rs tests/cli_completions.rs
git commit -m "feat(cli): add completions subcommand for 5 shells"
```

---

## Task 5: Write failing test for inline `Examples:` block in every subcommand

**Files:**
- Create: `tests/cli_help_examples.rs`

- [ ] **Step 1: Write the parametric test**

Create `tests/cli_help_examples.rs` with:

```rust
//! Asserts that every `qwick-memory <subcommand> --help` ends with an
//! `Examples:` block containing at least one `qwick-memory` invocation.

use assert_cmd::Command;

const SUBCOMMANDS: &[&str] = &[
    "save",
    "search",
    "list",
    "delete",
    "feedback",
    "doctor",
    "index-code",
    "symbol",
    "memory-for",
    "ast",
    "context",
    "walk",
    "conflicts",
    "supersedes",
    "prune",
    "gc",
    "install-hooks",
    "completions",
];

fn help_for(sub: &str) -> String {
    let out = Command::cargo_bin("qwick-memory")
        .expect("cargo_bin qwick-memory")
        .args([sub, "--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("help text is utf-8")
}

#[test]
fn every_subcommand_help_has_examples_block() {
    let mut missing: Vec<&str> = Vec::new();
    for sub in SUBCOMMANDS {
        let help = help_for(sub);
        let has_block = help.contains("Examples:");
        let has_invocation = help
            .lines()
            .skip_while(|l| !l.contains("Examples:"))
            .any(|l| l.contains("qwick-memory "));
        if !(has_block && has_invocation) {
            missing.push(sub);
        }
    }
    assert!(
        missing.is_empty(),
        "subcommands missing an Examples: block with a qwick-memory invocation: {missing:?}"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --test cli_help_examples`
Expected: FAIL — every subcommand except `completions` is in the `missing` list.

---

## Task 6: Add `EXAMPLES` to `save`

**Files:**
- Modify: `src/cli/save.rs`

- [ ] **Step 1: Add the const above the `Args` struct**

In `src/cli/save.rs`, immediately above `#[derive(ClapArgs, Debug)]\npub struct Args {`, insert:

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory save \"Use Postgres for analytics\" --kind decision --repo myrepo --tags db,postgres --quality 4
  echo \"Race in run_migration when run twice in <1s\" | qwick-memory save - --kind bug --repo myrepo";
```

- [ ] **Step 2: Attach `after_help` to the `Args` struct**

Change the existing line:

```rust
#[derive(ClapArgs, Debug)]
```

to:

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

- [ ] **Step 3: Verify `save --help` now ends with the block**

Run: `cargo run -q -- save --help | tail -5`
Expected: tail shows the `Examples:` block followed by two `qwick-memory save` lines.

---

## Task 7: Add `EXAMPLES` to `search`

**Files:**
- Modify: `src/cli/search.rs`

- [ ] **Step 1: Add the const + `after_help`**

In `src/cli/search.rs`, above `#[derive(ClapArgs, Debug)]`, insert:

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory search \"postgres migration race\"
  qwick-memory search \"what database do we use\" --limit 5 --json";
```

Then update the derive:

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 8: Add `EXAMPLES` to `list`

**Files:**
- Modify: `src/cli/list.rs`

- [ ] **Step 1: Add the const + `after_help`**

In `src/cli/list.rs`, above `#[derive(ClapArgs, Debug)]`, insert:

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory list --repo myrepo --kind decision
  qwick-memory list --json";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 9: Add `EXAMPLES` to `delete`

**Files:**
- Modify: `src/cli/delete.rs`

- [ ] **Step 1: Add the const + `after_help`**

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory delete a1b2c3d4
  qwick-memory delete a1b2c3d4 --json";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 10: Add `EXAMPLES` to `feedback`

**Files:**
- Modify: `src/cli/feedback.rs`

- [ ] **Step 1: Add the const + `after_help`**

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory feedback q-2026-05-17-001 --used a1b2c3d4,e5f6a7b8 --irrelevant 0011223344";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 11: Add `EXAMPLES` to `doctor` (unit variant)

**Files:**
- Modify: `src/cli/doctor.rs` (may be a no-arg module — see step 1)
- Modify: `src/cli/mod.rs`

`doctor` is a unit variant in `Cmd` (no `Args` struct). Attach `after_help` on the **variant** itself rather than on a derived struct.

- [ ] **Step 1: Add the const inside `src/cli/doctor.rs`**

At the top of `src/cli/doctor.rs`, below the existing `use` lines, add:

```rust
pub const EXAMPLES: &str = "\
Examples:
  qwick-memory doctor
  qwick-memory doctor --json";
```

- [ ] **Step 2: Annotate the variant in `src/cli/mod.rs`**

In `src/cli/mod.rs`, change:

```rust
    /// Report on the data directory and memory count.
    Doctor,
```

to:

```rust
    /// Report on the data directory and memory count.
    #[command(after_help = doctor::EXAMPLES)]
    Doctor,
```

---

## Task 12: Add `EXAMPLES` to `index-code`

**Files:**
- Modify: `src/cli/index_code.rs`

- [ ] **Step 1: Add the const + `after_help`**

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory index-code --root . --repo myrepo
  qwick-memory index-code --root /path/to/repo --incremental --quiet";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 13: Add `EXAMPLES` to `symbol`

**Files:**
- Modify: `src/cli/symbol.rs`

- [ ] **Step 1: Add the const + `after_help`**

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory symbol run_migration
  qwick-memory symbol \"parse frontmatter yaml\" --limit 10 --json";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 14: Add `EXAMPLES` to `memory-for`

**Files:**
- Modify: `src/cli/memory_for.rs`

- [ ] **Step 1: Add the const + `after_help`**

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory memory-for myrepo:src/db.rs:run_migration
  qwick-memory memory-for myrepo:src/db.rs";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 15: Add `EXAMPLES` to `ast`

**Files:**
- Modify: `src/cli/ast.rs`

- [ ] **Step 1: Add the const + `after_help`**

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory ast 'fn $NAME($$$ARGS) -> Result<$RET>' --lang rs --file src/db.rs
  qwick-memory ast 'tokio::spawn($$$)' --lang rs --file src/lib.rs --json";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 16: Add `EXAMPLES` to `context`

**Files:**
- Modify: `src/cli/context.rs`

- [ ] **Step 1: Add the const + `after_help`**

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory context run_migration --json
  qwick-memory context \"postgres migration race\" --depth 2";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 17: Add `EXAMPLES` to `walk`

**Files:**
- Modify: `src/cli/walk.rs`

- [ ] **Step 1: Add the const + `after_help`**

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory walk --from a1b2c3d4 --edge supersedes --depth 5 --json";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 18: Add `EXAMPLES` to `conflicts`

**Files:**
- Modify: `src/cli/conflicts.rs`

- [ ] **Step 1: Add the const + `after_help`**

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory conflicts a1b2c3d4
  qwick-memory conflicts a1b2c3d4 --json";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 19: Add `EXAMPLES` to `supersedes`

**Files:**
- Modify: `src/cli/supersedes.rs`

- [ ] **Step 1: Add the const + `after_help`**

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory supersedes e5f6a7b8 a1b2c3d4";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 20: Add `EXAMPLES` to `prune`

**Files:**
- Modify: `src/cli/prune.rs`

- [ ] **Step 1: Add the const + `after_help`**

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory prune --orphans
  qwick-memory prune --orphans --apply
  qwick-memory prune --low-value --below-quality 2 --unused-since 180 --apply";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 21: Add `EXAMPLES` to `gc` (unit variant)

**Files:**
- Modify: `src/cli/gc.rs`
- Modify: `src/cli/mod.rs`

`gc` is a unit variant in `Cmd`. Same pattern as Task 11.

- [ ] **Step 1: Add the const inside `src/cli/gc.rs`**

```rust
pub const EXAMPLES: &str = "\
Examples:
  qwick-memory gc
  qwick-memory gc --json";
```

- [ ] **Step 2: Annotate the variant**

In `src/cli/mod.rs`, change:

```rust
    /// Purge old entries from `memories/.trash/`.
    Gc,
```

to:

```rust
    /// Purge old entries from `memories/.trash/`.
    #[command(after_help = gc::EXAMPLES)]
    Gc,
```

---

## Task 22: Add `EXAMPLES` to `install-hooks`

**Files:**
- Modify: `src/cli/install_hooks.rs`

- [ ] **Step 1: Add the const + `after_help`**

```rust
const EXAMPLES: &str = "\
Examples:
  qwick-memory install-hooks --repo .
  qwick-memory install-hooks --repo /path/to/repo --force";
```

```rust
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
```

---

## Task 23: Verify the `--help` test now passes and run the umbrella gate

- [ ] **Step 1: Run the parametric help-examples test**

Run: `cargo nextest run --test cli_help_examples`
Expected: 1 PASS (all 18 subcommands have `Examples:` + a `qwick-memory ` invocation).

- [ ] **Step 2: Run the full umbrella gate**

Run: `bash scripts/check-all.sh`
Expected: exit 0.

- [ ] **Step 3: Commit**

```bash
git add src/cli/ tests/cli_help_examples.rs
git commit -m "feat(cli): add Examples: block to every subcommand --help"
```

---

## Task 24: Write `scripts/regen-cli-docs.sh`

**Files:**
- Create: `scripts/regen-cli-docs.sh`

- [ ] **Step 1: Write the script**

Create `scripts/regen-cli-docs.sh` with mode `0755`:

```bash
#!/usr/bin/env bash
# Regenerate docs/cli-reference.md from `qwick-memory <cmd> --help` output.
# This is the single source of truth for the CLI reference page.

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

STEP="regen-cli-docs"

OUT="${1:-$PROJECT_ROOT/docs/cli-reference.md}"

log_info "$STEP" "building release-quick binary"
run_cargo build --profile release-quick --locked --quiet

BIN="$PROJECT_ROOT/target/release-quick/qwick-memory"
[[ -x "$BIN" ]] || die "$STEP" "expected binary at $BIN"

SUBCOMMANDS=(
  save search list delete feedback doctor
  index-code symbol memory-for ast context walk
  conflicts supersedes prune gc install-hooks completions
)

{
  cat <<'HEADER'
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
HEADER

  "$BIN" --help

  echo '```'
  echo

  for sub in "${SUBCOMMANDS[@]}"; do
    echo "---"
    echo
    echo "## qwick-memory $sub"
    echo
    echo '```'
    "$BIN" "$sub" --help
    echo '```'
    echo
  done
} > "$OUT"

log_ok "$STEP" "wrote $OUT"
```

- [ ] **Step 2: Make the script executable**

Run: `chmod +x scripts/regen-cli-docs.sh`

- [ ] **Step 3: Sanity-run it**

Run: `bash scripts/regen-cli-docs.sh /tmp/qwick-cli-reference.md && head -40 /tmp/qwick-cli-reference.md`
Expected: header followed by the top-level `--help` output.

---

## Task 25: Regenerate `docs/cli-reference.md` from the binary

**Files:**
- Modify: `docs/cli-reference.md` (overwritten by the regen script)

- [ ] **Step 1: Regenerate**

Run: `bash scripts/regen-cli-docs.sh`
Expected: `scripts/regen-cli-docs.sh` overwrites `docs/cli-reference.md` and logs a single `[regen-cli-docs] wrote …` line.

- [ ] **Step 2: Spot-check the new markdown**

Run: `grep -c '^## qwick-memory ' docs/cli-reference.md`
Expected: `18` (one per subcommand).

Run: `grep -c '^Examples:' docs/cli-reference.md`
Expected: `18`.

---

## Task 26: Write `scripts/cli-docs-check.sh` (drift guard)

**Files:**
- Create: `scripts/cli-docs-check.sh`

- [ ] **Step 1: Write the script**

Create `scripts/cli-docs-check.sh` with mode `0755`:

```bash
#!/usr/bin/env bash
# Verify docs/cli-reference.md matches the output of scripts/regen-cli-docs.sh.

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

STEP="cli-docs-check"
TMP="$(mktemp -t qwick-cli-reference.XXXXXX)"
trap 'rm -f "$TMP"' EXIT

log_info "$STEP" "regenerating into $TMP"
bash "$HERE/regen-cli-docs.sh" "$TMP" >/dev/null

if ! diff -u "$PROJECT_ROOT/docs/cli-reference.md" "$TMP"; then
  printf "%s[%s]%s docs/cli-reference.md is stale; run %sbash scripts/regen-cli-docs.sh%s\n" \
    "$C_RED" "$STEP" "$C_RST" "$C_YLW" "$C_RST" 1>&2
  exit 1
fi

log_ok "$STEP" "docs/cli-reference.md is up to date"
```

- [ ] **Step 2: Make it executable**

Run: `chmod +x scripts/cli-docs-check.sh`

- [ ] **Step 3: Run it — expect green now (docs were just regenerated)**

Run: `bash scripts/cli-docs-check.sh`
Expected: exit 0, `[cli-docs-check] docs/cli-reference.md is up to date`.

- [ ] **Step 4: Sanity-check it fails on drift**

Run: `echo "stray line" >> docs/cli-reference.md && bash scripts/cli-docs-check.sh ; echo exit=$?`
Expected: exit 1, diff printed.

- [ ] **Step 5: Restore the file**

Run: `bash scripts/regen-cli-docs.sh && bash scripts/cli-docs-check.sh`
Expected: exit 0.

---

## Task 27: Wire `cli-docs-check.sh` into the umbrella gate

**Files:**
- Modify: `scripts/check-all.sh`

- [ ] **Step 1: Inspect the gate ordering**

Run: `grep -n 'sh"' scripts/check-all.sh`
Expected: a list of `bash "$HERE/<name>.sh"` (or similar) invocations.

- [ ] **Step 2: Append the new check**

In `scripts/check-all.sh`, after the `typos-check.sh` invocation (the last gate per CLAUDE.md), append:

```bash
bash "$HERE/cli-docs-check.sh"
```

Match the surrounding style (variable name and quoting).

- [ ] **Step 3: Run the umbrella gate**

Run: `bash scripts/check-all.sh`
Expected: exit 0, `cli-docs-check` appears in the output.

- [ ] **Step 4: Commit the docs + scripts**

```bash
git add scripts/regen-cli-docs.sh scripts/cli-docs-check.sh scripts/check-all.sh docs/cli-reference.md
git commit -m "docs(cli): generate cli-reference.md from binary, gate drift"
```

---

## Task 28: `scripts/install.sh` — add `--no-clean` flag and competing-install removal

**Files:**
- Modify: `scripts/install.sh`

- [ ] **Step 1: Read the current script**

Run: `cat scripts/install.sh`
Expected: 44-line script, single positional `--with-tools` arg parsing.

- [ ] **Step 2: Replace the arg parsing + add cleanup phase**

In `scripts/install.sh`, replace the existing block:

```bash
WITH_TOOLS=0
[[ "${1:-}" == "--with-tools" ]] && WITH_TOOLS=1

log_info "$STEP" "building release-quick binary"
```

with:

```bash
WITH_TOOLS=0
CLEAN=1
for arg in "$@"; do
  case "$arg" in
    --with-tools) WITH_TOOLS=1 ;;
    --no-clean)   CLEAN=0 ;;
    *) die "$STEP" "unknown argument: $arg (expected --with-tools or --no-clean)" ;;
  esac
done

if [[ "$CLEAN" -eq 1 ]]; then
  if command -v uv >/dev/null 2>&1 \
     && uv tool list 2>/dev/null | awk '{print $1}' | grep -qx 'qwick-memory'; then
    log_info "$STEP" "detected uv tool: qwick-memory — uninstalling"
    uv tool uninstall qwick-memory >/dev/null
  fi
  if command -v brew >/dev/null 2>&1 \
     && brew list --formula 2>/dev/null | grep -qx 'qwick-memory'; then
    log_info "$STEP" "detected brew formula: qwick-memory — uninstalling"
    brew uninstall qwick-memory >/dev/null
  fi
fi

log_info "$STEP" "building release-quick binary"
```

- [ ] **Step 3: Add a PATH-shadow warning after the existing install/log block**

Currently the script ends with the `case ":$PATH:"` block that warns when `$BIN_DIR` is missing. Below that block, append:

```bash
SHADOW=""
IFS=':' read -r -a PATH_PARTS <<< "$PATH"
for p in "${PATH_PARTS[@]}"; do
  candidate="$p/qwick-memory"
  if [[ -x "$candidate" && "$candidate" != "$BIN_PATH" ]]; then
    SHADOW="$candidate"
    break
  fi
done
if [[ -n "$SHADOW" ]]; then
  printf "%s[%s]%s warning: %s appears on PATH before %s — rehash your shell or remove the shadow\n" \
    "$C_YLW" "$STEP" "$C_RST" "$SHADOW" "$BIN_PATH"
fi
```

- [ ] **Step 4: Lint the script**

Run: `bash -n scripts/install.sh && command -v shellcheck >/dev/null && shellcheck scripts/install.sh || true`
Expected: exit 0 from `bash -n`. `shellcheck` warnings (if available) should be zero new findings against the lines you added.

- [ ] **Step 5: Manual verification matrix**

Run each on a scratch terminal and confirm the documented behaviour:

| Scenario | Command | Expected |
|---|---|---|
| Default run | `bash scripts/install.sh` | Builds + installs. Cleanup phase logs only when uv/brew has `qwick-memory`. PATH-shadow line appears when applicable. |
| Opt out | `bash scripts/install.sh --no-clean` | No `uv tool uninstall` / `brew uninstall` calls; install proceeds. |
| With tools | `bash scripts/install.sh --with-tools` | Existing behaviour preserved (sccache + hyperfine via brew). |
| Bad flag | `bash scripts/install.sh --bogus` | `die`s with "unknown argument: --bogus". |

Note the result of each in the PR description.

- [ ] **Step 6: Commit**

```bash
git add scripts/install.sh
git commit -m "build: auto-clean competing installs in install.sh (--no-clean opt-out)"
```

---

## Task 29: Final umbrella gate + smoke test

- [ ] **Step 1: Run the umbrella gate**

Run: `bash scripts/check-all.sh`
Expected: exit 0. All gates green, including `cli-docs-check`.

- [ ] **Step 2: Run the full nextest suite**

Run: `cargo nextest run --all-features`
Expected: exit 0. New tests in `tests/cli_completions.rs` and `tests/cli_help_examples.rs` are included.

- [ ] **Step 3: Smoke-test the binary**

Run:

```bash
cargo install --path . --profile release-quick --locked --force
qwick-memory --help | grep 'completions'
qwick-memory save --help | tail -5
qwick-memory completions fish | head -5
```

Expected:
- `--help` lists `completions` as a subcommand.
- `save --help` ends with the `Examples:` block.
- `completions fish` emits a fish completion script starting with a comment.

- [ ] **Step 4: Smoke-test the install script on the local box**

Run: `bash scripts/install.sh`
Expected:
- Cleanup logs appear only if uv tool / brew formula `qwick-memory` exists.
- Install succeeds with `release-quick` profile.
- PATH-shadow warning appears if `~/.local/bin/qwick-memory` (or similar) still exists.

---

## Self-Review

**Spec coverage:**
- Section 2 deliverable 1 (`completions` subcommand) → Tasks 1–4.
- Section 2 deliverable 2 (inline `Examples:` block) → Tasks 5–23.
- Section 2 deliverable 3 (generated `cli-reference.md` + drift guard) → Tasks 24–27.
- Section 2 deliverable 4 (`install.sh` auto-clean) → Task 28.
- Section 6 testing matrix (5 shells, every subcommand help, drift check, install manual matrix) → Tasks 2, 5, 26, 28.
- Section 7 binding rules (no duplication, modularity, ≤500 lines, no bypass, tests in `tests/`) — preserved throughout; new tests live in `tests/*.rs`, no `unwrap` / `println!` / `unsafe` introduced.

**Placeholder scan:** Every code step shows complete code. No "TBD" / "similar to Task N" / "add appropriate error handling". Tasks 6–22 repeat the `#[command(after_help = EXAMPLES)]` snippet for each subcommand explicitly.

**Type consistency:** All tasks use `const EXAMPLES: &str` (private) for `ClapArgs` structs and `pub const EXAMPLES: &str` for unit variants accessed from `mod.rs` (Tasks 11 and 21). `clap_complete::Shell` is the single value type for the `completions` subcommand.

---

## Plan complete

Plan saved to `docs/superpowers/plans/2026-05-18-cli-docs-and-install-cleanup-plan.md`.
