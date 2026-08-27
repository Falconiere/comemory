# Getting started

A single, linear walk through `comemory` — install the binary, save your first
memory, search it back, then index a repo and search code. Follow it top to
bottom once and you'll have a working memory store plus a code index. Where a
step has variations, this guide picks one happy path and links out.

> **Try it in a sandbox.** The save/search steps work against any data
> directory. To experiment without touching your real store, point `comemory`
> at a throwaway directory first:
>
> ```bash
> export COMEMORY_DATA_DIR=$(mktemp -d)
> ```
>
> Steps 4–5 (`index-code` / `search-code`) need a real git repo with code, so
> run those from inside a checkout.

---

## 1. Install

Pick whichever channel fits your machine. The one-liner is the shortest path on
Apple Silicon macOS and glibc ≥ 2.35 Linux — it redirects to the latest release's
shell installer. Those are the only prebuilt targets, and the glibc floor is
enforced by the installer itself. An Intel Mac or a musl distro gets
`there isn't a download for your platform <triple>`; an older glibc (Debian
bullseye 2.31, Ubuntu 20.04 2.31, RHEL 9 / Amazon Linux 2023 2.34) matches the
gnu triple but fails the gate and gets `no compatible downloads were found for
your platform <triple>` instead. Both need the source install below.

```bash
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL \
  https://get.comemory.io/pkg/comemory/install | bash
```

That installer runs under `/bin/sh` — cargo-dist generates it with a
`#!/bin/sh` shebang and targets dash/ash (it uses `local`, which POSIX does not
define) — so `| sh` is equally correct and is the one to use on a glibc image
that ships no bash. (Only gnu targets are published and only for glibc ≥ 2.35, so
Alpine or an older-glibc base image needs the source install either way.)

Scripting the install — a Dockerfile `RUN`, or any CI whose shell lacks
`pipefail` — download and run in two steps instead. A pipeline reports only the
shell's exit status, so if the fetch ever 404s, `curl … | sh` still exits 0 and
the build carries on with nothing installed, surfacing much later as
`comemory: command not found` (a GitHub Actions step catches this only with an
explicit `shell: bash` — the implicit default is `bash -e`, no pipefail — and
`/bin/sh -c` in a Dockerfile never does):

```bash
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL \
  https://get.comemory.io/pkg/comemory/install -o install.sh && sh install.sh
```

In a **Dockerfile** add `COMEMORY_INSTALL_DIR=/usr/local` to that second command
as well (`… && COMEMORY_INSTALL_DIR=/usr/local sh install.sh`). Only there —
elsewhere it needs write access to `/usr/local`, which a non-root CI user does
not have. By default the installer puts the binary in `$CARGO_HOME/bin`
(`$HOME/.cargo/bin` when `CARGO_HOME` is unset, which is the usual case in a base
image) and makes it reachable by editing rc files. It appends `. "…/env"` to
**every** one of `~/.profile`, `~/.bashrc`, `~/.bash_profile`, `~/.bash_login`
that exists (not just the first), to whichever of `~/.zshrc`, `~/.zshenv` comes
first, and `source …/env.fish` to fish's `conf.d/comemory.env.fish`. Where none
of a list exists it creates the first entry, so a bare image ends up with new rc
files rather than none — worth knowing if you ever need to undo it. That is the default path's problem, and the reason for
`COMEMORY_INSTALL_DIR` above: a later `RUN` and the container entrypoint read
none of those rc files, so without the flag `comemory` ends up installed and
still not on `PATH`.

Homebrew is as quick, on macOS and Linuxbrew:

```bash
brew install Falconiere/tap/comemory
```

From a local checkout (comemory is **not** published to crates.io, so
`cargo install --path .` builds from source):

```bash
git clone https://github.com/Falconiere/comemory && cd comemory
cargo install --path .
```

Prefer a prebuilt binary? Tarballs for macOS (aarch64) and Linux (aarch64,
x86_64) are attached to every
[GitHub Release](https://github.com/Falconiere/comemory/releases). The curl
installer — the same script the one-liner above redirects to — fetches the right
one and drops it in `$CARGO_HOME/bin` (or `$HOME/.cargo/bin`). It installs no
shell completions:

```bash
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -LsSf \
  https://github.com/Falconiere/comemory/releases/latest/download/comemory-installer.sh \
  | sh
```

No install channel sets up shell completions — Homebrew, the curl installer and
`cargo install` alike. Generate them yourself with
`comemory completions <bash|zsh|fish|powershell>`.

Verify the install — `comemory doctor` checks the data directory and the SQLite
mirror:

```bash
comemory doctor
```

You should see a health report naming the data directory and confirming the
SQLite store is reachable. If it prints a clean report, you're ready.

---

## 2. Save your first memory

A memory is a short note with a kind, an optional repo, and tags:

```bash
comemory save "Use Postgres for analytics, not ClickHouse — see ADR-14" \
  --kind decision --repo demo --tags db,analytics
```

`--kind` is one of `decision`, `bug`, `convention`, `discovery`, `pattern`, or
`note` (defaults to `note`). `--quality` takes `1..=5` and defaults to `3`.

Markdown is the source of truth: the note lands as a file under
`~/.comemory/memories/{id}-{slug}.md` (or under `$COMEMORY_DATA_DIR/memories/`
if you set that override). The SQLite index is just a mirror you can always
rebuild from the markdown.

---

## 3. Search memories

Query the memory index in natural language — identifier tokens
(camelCase/snake_case) are split automatically:

```bash
comemory search "postgres analytics"
```

The TTY view prints ranked hits. Add `--json` for a machine-readable result:
the output is a `Page` envelope shaped
`{ items, limit, offset, total, has_more }`, so scripts can page through results
and detect when more remain.

```bash
comemory search "postgres analytics" --json
```

---

## 4. Index code

Run this from **inside a git repo** (blob OIDs drive the incremental skip path).
`comemory index-code` walks the tree, extracts symbols, and upserts them into
the code index. It understands Rust, TypeScript, JavaScript, Python, and Go.

```bash
comemory index-code --repo demo --path .
```

`--repo` is the label stored on each symbol row (reuse it later to scope code
searches); `--path` is the working-tree root to walk. To emit JSONL for an
external embedder instead of writing rows, add `--extract`.

---

## 5. Search code and context

Search the code index — ranking blends BM25 lexical relevance with graph
priors (PageRank, recency, working-set affinity):

```bash
comemory search-code "parse frontmatter"
```

Scope it with `--repo demo` or `--lang rust` when you want a narrower slice.

For the headline view that fuses both halves — the matching code symbol plus the
memories attached to it, with code refs ranked by graph priors — use `context`:

```bash
comemory context "frontmatter" --repo demo
```

The relations themselves are searchable too. `comemory edges` reads the graph
lexically, so "what replaced that decision?" or "what imports the parser?" have
word-shaped answers instead of requiring an id:

```bash
comemory edges "supersedes frontmatter"
```

Each hit prints as `src —rel→ dst`, where a memory endpoint renders as its kind
plus slug and a code endpoint as its path.

That's the loop: save what you learn, search it back, and let the code index and
graph surface the symbols and memories that matter.

---

## Next steps

You're productive. Branch out as needed:

- [Bring-your-own vectors](guides/byo-vectors.md) — add dense embeddings for
  semantic search.
- [Automatic re-indexing](guides/auto-reindex.md) — keep the code index fresh
  via lazy refresh or git hooks.
- [Ranking and eval](guides/ranking-and-eval.md) — score retrieval, mine
  expansions, and tune the blend.
- [Serve the web viewer](guides/serve-web.md) — explore the graph and edit code
  in the browser.
- [Prune and gc](guides/prune-and-gc.md) — retire stale memories and purge
  telemetry.
- [CLI reference](cli-reference.md) — every subcommand and flag.
- [Architecture](architecture.md) — how the store, edges, and ranking fit
  together.
