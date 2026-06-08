# Changelog

## 0.2.0 — 2026-06-08 (Lightweight refactor)

### Breaking
- Dropped `comemory serve` (axum web UI).
- Dropped the in-process embedder. Embedding is now the caller's
  responsibility; pass vectors via `--vector` or `--vector-stdin`.
- `~/.comemory/lancedb/` and `~/.comemory/kuzu/` directories are
  ignored. Run `comemory rebuild` to populate `~/.comemory/comemory.db`
  from `memories/*.md`.
- `--lang` on `comemory ast` now accepts only `rust`, `typescript`,
  `javascript`, `python`, `go`.

### Added
- `comemory ingest-code` reads pre-embedded JSONL into `code_symbols`
  and `code_vec`.
- `comemory rebuild` drops and reconstructs `comemory.db` from
  markdown.
- `scripts/comemory-embed.sh` — sample Ollama wrapper for the BYO
  contract.

### Changed
- Single `~/.comemory/comemory.db` SQLite file backs all storage
  (memories, FTS5, sqlite-vec, edges, stats).
- Release binary size: 117 MB → ~8 MB (after dropping the in-process
  embedder/lancedb/kuzu and trimming `ast-grep-language` to the
  rust/typescript/javascript/python/go tree-sitter parsers).
