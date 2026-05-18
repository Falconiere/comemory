//! Arrow schemas for qwick's LanceDB tables. Embedding dimension is
//! parameterized at build time so the same table layout works for any model.
//!
//! Two tables live here:
//! - `memory_chunks` — prose memories, vector keyed on `id`
//! - `code_chunks`   — extracted code symbols, vector keyed on `qualified`

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema};

/// Name of the LanceDB table that stores memory embeddings and their metadata.
pub const MEMORY_TABLE: &str = "memory_chunks";

/// Name of the LanceDB table that stores code-symbol embeddings.
pub const CODE_TABLE: &str = "code_chunks";

/// Build the arrow schema for `memory_chunks` with vector column dimensionality `dim`.
///
/// Columns:
/// - `id` (Utf8): memory id (PK on merge_insert)
/// - `body` (Utf8): plain-text memory body
/// - `kind` (Utf8): one of decision/bug/convention/discovery/pattern/note
/// - `repo` (Utf8): originating repo name
/// - `tags` (Utf8): comma-separated tag list (denormalized for fast filtering)
/// - `created` (Utf8): ISO-8601 UTC timestamp
/// - `quality` (Int32): 1..=5 rating
/// - `content_hash` (Utf8): sha-256 of body trimmed bytes
/// - `embedding` (FixedSizeList\<Float32, dim\>): vector representation
pub fn memory_schema(dim: usize) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("body", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("repo", DataType::Utf8, false),
        Field::new("tags", DataType::Utf8, false),
        Field::new("created", DataType::Utf8, false),
        Field::new("quality", DataType::Int32, false),
        Field::new("content_hash", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dim as i32,
            ),
            false,
        ),
    ]))
}

/// Build the arrow schema for `code_chunks` with vector column dimensionality `dim`.
///
/// Columns:
/// - `qualified` (Utf8): `<repo>:<path>:<symbol>` — primary key on merge_insert
/// - `snippet` (Utf8): source text for the extracted symbol
/// - `language` (Utf8): lower-case language name (rust/python/typescript/javascript)
/// - `file` (Utf8): `<repo>:<path>` — denormalized for repo+path filtering
/// - `symbol_kind` (Utf8): function/struct/enum/trait/class
/// - `ast_hash` (Utf8): sha-256 hex of normalized snippet bytes
/// - `embedding` (FixedSizeList\<Float32, dim\>): jina-code vector representation
pub fn code_schema(dim: usize) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("qualified", DataType::Utf8, false),
        Field::new("snippet", DataType::Utf8, false),
        Field::new("language", DataType::Utf8, false),
        Field::new("file", DataType::Utf8, false),
        Field::new("symbol_kind", DataType::Utf8, false),
        Field::new("ast_hash", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dim as i32,
            ),
            false,
        ),
    ]))
}
