//! Declared schema — the code index: `code_symbols`, its `code_fts` /
//! `code_vec` virtual tables, the per-file `indexed_files` cursor and the
//! per-repo `repo_marker` cursor.

use toolu_orm::core::column::{Integer, Real, Text, Vector};
use toolu_orm::{fts5_table, table, vec0_table};

/// `code_symbols`: one row per extracted symbol (or cAST chunk, via
/// `parent_id`). The live table declares its uniqueness as a table-level
/// `UNIQUE (repo, path, symbol, line_start)`; toolu-orm has no table-level
/// constraint form, so it is declared as the equivalent unique index. The
/// fidelity test (`schema_fidelity_ordinary_tables_match_the_live_database`)
/// compares unique column-sets, which both forms produce, so the
/// equivalence is proven against the real database, not assumed.
#[table(name = "code_symbols")]
#[index("idx_code_repo_path", repo, path)]
#[index("idx_code_blob", blob_oid)]
#[index("idx_code_simhash", simhash)]
#[unique_index("uq_code_symbols_location", repo, path, symbol, line_start)]
pub struct CodeSymbols {
    /// Rowid alias.
    #[column(primary_key)]
    pub id: Integer,
    /// Repo label.
    #[column(not_null)]
    pub repo: Text,
    /// Path relative to the repo root.
    #[column(not_null)]
    pub path: Text,
    /// Git blob OID of the file at index time (incremental reindex key).
    #[column(not_null)]
    pub blob_oid: Text,
    /// Qualified symbol name.
    #[column(not_null)]
    pub symbol: Text,
    /// `function` / `struct` / … as the extractor reports it.
    #[column(not_null)]
    pub kind: Text,
    /// `rust` / `typescript` / ….
    #[column(not_null)]
    pub lang: Text,
    /// First line (1-based).
    #[column(not_null)]
    pub line_start: Integer,
    /// Last line (1-based, inclusive).
    #[column(not_null)]
    pub line_end: Integer,
    /// Raw source text, for FTS and display.
    #[column(not_null)]
    pub snippet: Text,
    /// 64-bit SimHash bit pattern of the snippet tokens.
    #[column(not_null)]
    pub simhash: Integer,
    /// RFC3339 index time.
    #[column(not_null)]
    pub indexed_at: Text,
    /// Access tracking (v4).
    #[column(not_null, default = "0")]
    pub access_count: Integer,
    /// Last access, RFC3339 (v4).
    pub last_accessed: Text,
    /// Materialized PageRank (v6).
    #[column(not_null, default = "0.0")]
    pub rank_score: Real,
    /// Parent symbol of a cAST chunk row (v6).
    pub parent_id: Integer,
}

/// `code_fts`: lexical index over symbol names, snippets and path tokens,
/// split by the custom `identifier` tokenizer.
#[fts5_table(name = "code_fts", tokenize = "identifier")]
pub struct CodeFts {
    /// Owning `code_symbols.id`, stored but not searchable.
    #[column(unindexed)]
    pub symbol_id: Text,
    /// Qualified symbol name.
    pub symbol: Text,
    /// Raw source text.
    pub snippet: Text,
    /// Path split into identifier tokens.
    pub path_tokens: Text,
}

/// `code_vec`: BYO code embeddings, 768 dims, cosine.
#[vec0_table(name = "code_vec")]
pub struct CodeVec {
    /// Owning `code_symbols.id`.
    #[column(primary_key)]
    pub symbol_id: Integer,
    /// Unit-length embedding.
    #[column(dim = 768, distance_metric = "cosine")]
    pub embedding: Vector,
}

/// `indexed_files`: the blob OID each indexed file was last extracted at,
/// so an incremental `index-code` skips unchanged files.
#[table(name = "indexed_files")]
#[primary_key(repo, path)]
pub struct IndexedFiles {
    /// Repo label.
    #[column(not_null)]
    pub repo: Text,
    /// Path relative to the repo root.
    #[column(not_null)]
    pub path: Text,
    /// Git blob OID at index time.
    #[column(not_null)]
    pub blob_oid: Text,
    /// RFC3339 index time.
    #[column(not_null)]
    pub indexed_at: Text,
}

/// `repo_marker`: one row per indexed repo — the HEAD the index was built
/// at, the co-change mining cursor, the working-tree root `serve` resolves
/// file ids against, and the console's archived flag.
#[table(name = "repo_marker")]
pub struct RepoMarker {
    /// Repo label.
    #[column(primary_key)]
    pub repo: Text,
    /// HEAD commit at the last index.
    pub last_head: Text,
    /// RFC3339 time of the last index.
    pub last_indexed_at: Text,
    /// Last commit `graph::cochange` mined (v6).
    pub last_mined_commit: Text,
    /// Absolute working-tree root captured at index time (v7).
    pub root_path: Text,
    /// `1` when the console stopped indexing the repo (v15).
    #[column(not_null, default = "0")]
    pub archived: Integer,
}
