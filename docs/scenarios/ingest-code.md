# `comemory ingest-code`

Read pre-embedded JSONL on stdin (the shape `index-code --extract` emits,
plus `embedding`) and upsert `code_symbols` + `code_fts` + `code_vec`.
No command-local flags — input is the stream.

**Runnable tests:** `tests/cli__ingest_code.rs`, `tests/cli__ingest_code_2.rs`,
`tests/cli_scenario_vectors.rs`

**HTTP:** `POST /api/v1/code/ingest` — covered by `tests/serve_scenario_vectors.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._ JSONL on stdin.

## Flags

_None besides globals._

## Scenarios

### ingest-code-01 Happy path

- **Flags:** _(stdin)_
- **Setup:** one JSONL row with a 768-dim `embedding`
- **Command:**

```bash
comemory ingest-code <<'EOF'
{"repo":"sample","path":"src/lib.rs","blob_oid":"0"*40,"symbol":"run_migration","kind":"function","lang":"rust","line_start":1,"line_end":3,"snippet":"fn run_migration() {}","simhash":0,"embedding":[...]}
EOF
```

- **Expect:** one `code_symbols` row and one `code_vec` row.
- **Covered by:** `tests/cli__ingest_code.rs::ingest_code_inserts_row_with_supplied_embedding`

### ingest-code-02 Extract then ingest

- **Flags:** _(stdin from extract)_
- **Setup:** `index-code --extract`, splice 768-dim embeddings
- **Command:** `comemory index-code --extract … | embed | comemory ingest-code`
- **Expect:** `search-code --vector-stdin` hits the ingested symbol.
- **Covered by:** `tests/cli_scenario_vectors.rs::extract_embed_ingest_search_code_vector`

### ingest-code-03 Wrong dim / malformed stream

- **Flags:** _(stdin)_
- **Expect:** wrong-dim vector fails; a malformed mid-stream row rolls back
  the whole ingest; conflicting `blob_oid` for the same path is rejected.
- **Covered by:** `tests/cli__ingest_code_2.rs`
