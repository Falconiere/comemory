# Comemory CLI Guide

Comemory fuses developer memory, semantic code search, and AST patterns
into one local-first binary. This guide walks through installing the
tool, indexing a repository, and running your first searches.

## Installation

Install the binary with cargo:

```bash
cargo install --path .
```

Verify the install succeeded:

```bash
comemory doctor
```

### Homebrew

Alternatively, install via the Falconiere tap:

```bash
brew install Falconiere/tap/comemory
```

## Indexing a repository

Before searching, index the code graph:

```bash
comemory index-code
```

This walks the repository, extracts symbols via ast-grep, and writes
them into the local SQLite store. Re-run it any time the tree changes;
lazy reindexing keeps things fresh automatically.

### Supported languages

The extractor currently understands:

- Rust
- TypeScript
- JavaScript
- Python
- Go

### Ignoring paths

Add a `.gitignore`-style pattern to skip vendored directories:

```
node_modules/
# build artifacts
target/
```

## Searching

Once indexed, two search surfaces are available.

### Memory search

```bash
comemory search "why did we pick sqlite-vec"
```

### Code search

```bash
comemory search-code "token bucket rate limiter"
```

Both surfaces support `--json` for machine-readable output and `--k`
to control how many results come back.

## Troubleshooting

If results look stale, check the doctor output first:

```bash
comemory doctor
```

A stale index almost always means the lazy reindex debounce window
hasn't elapsed yet — force it with:

```bash
comemory index-code --force
```

## Next steps

- Read the architecture doc for the retrieval pipeline internals.
- Try the TUI explorer with `comemory tui`.
- Wire an embedder for hybrid search — see the BYO-vector guide.
