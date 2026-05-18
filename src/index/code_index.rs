//! LanceDB-backed vector index for code symbols. Walks a repo via `ignore::Walk`,
//! extracts symbols per file with `ast::extract`, embeds snippets with
//! `Embedder::jina_code`, and upserts into `code_chunks`. Keyed on a
//! `<repo>:<path>:<symbol>` qualified string so re-indexing the same repo
//! merges in-place instead of duplicating rows.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use ignore::Walk;
use lancedb::Connection;
use sha2::{Digest, Sha256};

use crate::ast::{extract, Lang};
use crate::index::embedder::Embedder;
use crate::index::schema::{code_schema, CODE_TABLE};
use crate::prelude::*;

/// LanceDB connection plus the cached arrow schema we encode rows against.
pub struct CodeIndex {
    conn: Connection,
    schema: Arc<Schema>,
}

/// One row of the `code_chunks` table prior to embedding. Built by
/// `index_repo` while walking the source tree, then collated into a single
/// arrow `RecordBatch` along with the parallel embedding vectors.
#[derive(Debug, Clone)]
pub struct CodeChunk {
    /// `<repo>:<path>:<symbol>` — primary key on `merge_insert`.
    pub qualified: String,
    /// Source text of the extracted symbol.
    pub snippet: String,
    /// Lowercase language tag (`rust`, `python`, `typescript`, `javascript`).
    pub language: String,
    /// `<repo>:<path>` — denormalized for repo+path filtering.
    pub file: String,
    /// Symbol kind: `function`, `struct`, `enum`, `trait`, `class`.
    pub symbol_kind: String,
    /// sha-256 hex of normalized snippet bytes; used for incremental skip.
    pub ast_hash: String,
}

impl CodeIndex {
    /// Open (or create) the LanceDB database at `dir`. `dim` MUST match the
    /// embedder used by `index_repo`.
    pub async fn open(dir: impl AsRef<Path>, dim: usize) -> Result<Self> {
        let uri = dir.as_ref().to_string_lossy().to_string();
        let conn = lancedb::connect(&uri)
            .execute()
            .await
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self {
            conn,
            schema: code_schema(dim),
        })
    }

    /// Borrow the inner LanceDB connection. Used by retrieval-layer code that
    /// needs to open the `code_chunks` table directly for vector queries
    /// without re-implementing the connection plumbing.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Walk `repo_root`, extract symbols from every supported source file,
    /// embed snippets with `emb`, and upsert into `code_chunks`. Returns the
    /// number of rows written (0 if the repo has no indexable symbols).
    ///
    /// Re-runs are idempotent: rows are merged on `qualified`, so unchanged
    /// symbols overwrite themselves rather than duplicate. Caller can detect
    /// unchanged snippets cheaply via `ast_hash` once incremental skip lands.
    pub async fn index_repo(
        &self,
        repo_root: &Path,
        repo: &str,
        emb: &mut Embedder,
    ) -> Result<usize> {
        let chunks = collect_chunks(repo_root, repo)?;
        if chunks.is_empty() {
            return Ok(0);
        }
        let snippets: Vec<String> = chunks.iter().map(|c| c.snippet.clone()).collect();
        let vecs = emb.embed_many(snippets)?;
        if vecs.len() != chunks.len() {
            return Err(Error::Other(format!(
                "embedder returned {} vectors for {} chunks",
                vecs.len(),
                chunks.len()
            )));
        }
        let batch = self.batch(&chunks, &vecs)?;
        let schema = self.schema.clone();
        let names = self
            .conn
            .table_names()
            .execute()
            .await
            .map_err(|e| Error::Other(e.to_string()))?;

        if names.iter().any(|n| n == CODE_TABLE) {
            let batches = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
            let tbl = self
                .conn
                .open_table(CODE_TABLE)
                .execute()
                .await
                .map_err(|e| Error::Other(e.to_string()))?;
            let mut merge = tbl.merge_insert(&["qualified"]);
            merge.when_matched_update_all(None);
            merge.when_not_matched_insert_all();
            merge
                .execute(Box::new(batches))
                .await
                .map_err(|e| Error::Other(e.to_string()))?;
        } else {
            let batches = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
            self.conn
                .create_table(CODE_TABLE, Box::new(batches) as Box<_>)
                .execute()
                .await
                .map_err(|e| Error::Other(e.to_string()))?;
        }
        Ok(chunks.len())
    }

    /// Encode a slice of `CodeChunk` + parallel embedding vectors into a
    /// single-row-per-chunk `RecordBatch` matching `code_schema`.
    fn batch(&self, chunks: &[CodeChunk], vecs: &[Vec<f32>]) -> Result<RecordBatch> {
        if chunks.is_empty() || vecs.is_empty() {
            return Err(Error::Other("batch called with no chunks".into()));
        }
        let dim = vecs[0].len();
        let flat: Vec<f32> = vecs.iter().flatten().copied().collect();
        let item_field = Arc::new(Field::new("item", DataType::Float32, true));
        let values: Arc<dyn Array> = Arc::new(Float32Array::from(flat));
        let emb_array = FixedSizeListArray::try_new(item_field, dim as i32, values, None)
            .map_err(|e| Error::Other(e.to_string()))?;

        let qualified: Vec<String> = chunks.iter().map(|c| c.qualified.clone()).collect();
        let snippet: Vec<String> = chunks.iter().map(|c| c.snippet.clone()).collect();
        let language: Vec<String> = chunks.iter().map(|c| c.language.clone()).collect();
        let file: Vec<String> = chunks.iter().map(|c| c.file.clone()).collect();
        let symbol_kind: Vec<String> = chunks.iter().map(|c| c.symbol_kind.clone()).collect();
        let ast_hash: Vec<String> = chunks.iter().map(|c| c.ast_hash.clone()).collect();

        RecordBatch::try_new(
            self.schema.clone(),
            vec![
                Arc::new(StringArray::from(qualified)),
                Arc::new(StringArray::from(snippet)),
                Arc::new(StringArray::from(language)),
                Arc::new(StringArray::from(file)),
                Arc::new(StringArray::from(symbol_kind)),
                Arc::new(StringArray::from(ast_hash)),
                Arc::new(emb_array),
            ],
        )
        .map_err(|e| Error::Other(e.to_string()))
    }
}

/// Walk `repo_root` with `ignore::Walk` (respects `.gitignore` + `.ignore`)
/// and return every supported source file path. Public so callers in later
/// tasks can reuse the same walk for graph extraction or hashing.
pub fn iter_files(root: &Path) -> Vec<PathBuf> {
    Walk::new(root)
        .flatten()
        .map(|d| d.path().to_path_buf())
        .filter(|p| p.is_file())
        .collect()
}

/// Walk the repo, run `ast::extract` per file, and collect raw chunks. Files
/// with unsupported extensions or unreadable bytes are silently skipped so a
/// single malformed file cannot abort the whole index pass.
fn collect_chunks(repo_root: &Path, repo: &str) -> Result<Vec<CodeChunk>> {
    let mut chunks = Vec::new();
    for path in iter_files(repo_root) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let Some(lang) = Lang::from_extension(&ext) else {
            continue;
        };
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        let syms = extract(lang, &src)?;
        let rel = path
            .strip_prefix(repo_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        for s in syms {
            let qualified = format!("{repo}:{rel}:{}", s.name);
            let ast_hash = sha256_hex(s.snippet.as_bytes());
            chunks.push(CodeChunk {
                qualified,
                snippet: s.snippet,
                language: s.language,
                file: format!("{repo}:{rel}"),
                symbol_kind: s.kind,
                ast_hash,
            });
        }
    }
    Ok(chunks)
}

/// sha-256 of `bytes` as a lower-case hex string. Used for `ast_hash` on
/// extracted snippets so incremental indexing in later tasks can short-
/// circuit when a symbol's snippet hasn't changed.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}
