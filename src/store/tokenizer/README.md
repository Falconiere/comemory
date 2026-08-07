# store/tokenizer/

**What belongs here:** the custom FTS5 identifier tokenizer — pure text
splitting logic (camelCase / snake_case / digit boundary awareness) and the
`unsafe` FFI registration that exposes it to SQLite as `tokenize =
'identifier'` via the raw `fts5_api`.

**What does NOT belong here:** FTS5 query construction. `store::fts` and
`store::fts_memory` build and run `MATCH` queries against columns that use
this tokenizer; this folder only controls how text is split into tokens at
index time.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `ffi.rs` | `register` | Registers the `identifier` FTS5 tokenizer on a connection via the raw `fts5_api`; the three `unsafe extern "C" fn` callbacks (`x_create`/`x_delete`/`x_tokenize`) each carry a `// SAFETY:` comment |
| `split.rs` | `SplitToken` | Splits text into FTS tokens with camelCase/snake_case/digit boundary awareness |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/store/tokenizer.rs` (`pub
mod <name>;`) and callers import concrete paths.
