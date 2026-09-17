# ast/

**What belongs here:** ast-grep-based symbol extraction and pattern search —
turning source text into `ExtractedSymbol`s and running user-supplied AST
patterns against a file, for the five supported languages (rust, typescript,
javascript, python, go).

**What does NOT belong here:** persisting extracted symbols to SQLite (that's
`store::code_row`), resolving per-language import statements (that's
`graph::imports`), and the `comemory ast` CLI argument shape (that's
`cli::ast`, which calls into `pattern::find`).

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `chunk.rs` | `Chunk` | cAST-style greedy chunking of oversized AST nodes into line-budgeted child chunks |
| `extractor.rs` | `ExtractedSymbol` | Symbol extraction via `ast-grep-core` patterns, one pattern set per language |
| `languages.rs` | `Lang` | Registry of compiled-in ast-grep languages and extension-to-language detection |
| `pattern.rs` | `find` | User-facing ast-grep pattern search over a single source file (`comemory ast`) |
| `pattern_cache.rs` | `cached` | Process-global, compile-once cache of ast-grep `Pattern`s |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/ast.rs` (`pub mod <name>;`)
and callers import concrete paths.
