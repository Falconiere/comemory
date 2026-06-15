//! Synthesized bench corpus rooted in a tempdir: a migrated `comemory.db`
//! seeded with N memory rows and M code-symbol rows (each with a vector)
//! through the real public store API — no mocks, so the retrieval and
//! store benches measure a populated index, not an empty query.
//!
//! `vectors.rs` is nested via `#[path]` (rather than a sibling `mod`) so
//! this file is self-contained for every bench binary that includes it.

#[path = "vectors.rs"]
pub mod vectors;

use comemory::config::{Config, Paths};
use comemory::memory::frontmatter::{Frontmatter, Kind, References, Relations};
use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::{connection, fts, memory_row, vector};
use rusqlite::Connection;
use tempfile::TempDir;
use time::OffsetDateTime;
use vectors::vector;

/// Memory vector dimension — must match the `memory_vec` vec0 DDL.
pub const MEMORY_DIM: usize = 1024;
/// Code vector dimension — must match the `code_vec` vec0 DDL.
pub const CODE_DIM: usize = 768;

/// Small shared vocabulary mixed into memory bodies and code snippets so
/// FTS queries (`"postgres pool"`, `"tokenizer ranking"`) land real hits.
const VOCAB: [&str; 8] = [
    "postgres",
    "pool",
    "tokenizer",
    "fts5",
    "ranking",
    "vector",
    "migration",
    "retrieval",
];

/// A built corpus rooted in a `TempDir`: a ready `Connection`, the default
/// `Config`, and the seeded memory ids / code symbol rowids.
pub struct BenchCorpus {
    /// Keeps the tempdir (and its `comemory.db`) alive for the bench.
    pub _tmp: TempDir,
    /// Migrated connection with `sqlite-vec` loaded.
    pub conn: Connection,
    /// Hermetic default config (no env / file layering).
    pub cfg: Config,
    /// Ids of the seeded memory rows, insertion order.
    pub mem_ids: Vec<String>,
    /// Rowids of the seeded code symbols, insertion order.
    pub code_ids: Vec<i64>,
}

/// Build `n_memories` memory rows and `m_symbols` code symbols through the
/// public store API, each carrying a deterministic vector. Heavy enough to
/// be built once outside the criterion timer.
pub fn build_corpus(n_memories: usize, m_symbols: usize) -> BenchCorpus {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::new(tmp.path());
    paths.ensure_dirs().unwrap();
    let conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();
    let mem_ids = (0..n_memories).map(|i| seed_memory(&conn, i)).collect();
    let code_ids = (0..m_symbols).map(|j| seed_code(&conn, j)).collect();
    BenchCorpus {
        _tmp: tmp,
        conn,
        cfg,
        mem_ids,
        code_ids,
    }
}

/// Insert memory `i` (row + FTS + vector) and return its synthetic id.
fn seed_memory(conn: &Connection, i: usize) -> String {
    let id = format!("{i:08x}");
    let body = memory_body(i);
    let fm = Frontmatter {
        id: id.clone(),
        kind: Kind::Note,
        repo: "bench".to_string(),
        tags: vec![VOCAB[i % VOCAB.len()].to_string()],
        author: "bench".to_string(),
        created: OffsetDateTime::UNIX_EPOCH,
        quality: 3,
        schema: 1,
        content_hash: id.clone(),
        references: References::default(),
        relations: Relations::default(),
    };
    let slug = format!("mem-{i}");
    let md_path = format!("memories/{id}-{slug}.md");
    memory_row::insert(conn, &fm, &body, &slug, &md_path, &fm.tags).unwrap();
    vector::insert_memory(conn, &fm.id, &vector(&fm.id, MEMORY_DIM)).unwrap();
    id
}

/// Insert code symbol `j` (row + FTS + vector) and return its rowid.
fn seed_code(conn: &Connection, j: usize) -> i64 {
    let path = format!("src/f{j}.rs");
    let symbol = format!("fn_{j}");
    let snippet = code_snippet(j);
    let id = code_row::insert(
        conn,
        &CodeSymbolRow {
            repo: "bench",
            path: &path,
            blob_oid: "oid",
            symbol: &symbol,
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 10,
            snippet: &snippet,
            simhash: 0,
            parent_id: None,
        },
    )
    .unwrap();
    fts::index_code(conn, id, &symbol, &snippet, &path).unwrap();
    vector::insert_code(conn, id, &vector(&format!("c{j}"), CODE_DIM)).unwrap();
    id
}

/// Body text for memory `i`, mixing the shared vocabulary so FTS queries hit.
fn memory_body(i: usize) -> String {
    let a = VOCAB[i % VOCAB.len()];
    let b = VOCAB[(i + 3) % VOCAB.len()];
    let c = VOCAB[(i + 5) % VOCAB.len()];
    format!("memory {i} about {a} {b} {c} connection handling and index tuning")
}

/// Snippet text for code symbol `j`, mixing vocabulary into a tiny fn body.
fn code_snippet(j: usize) -> String {
    let a = VOCAB[j % VOCAB.len()];
    let b = VOCAB[(j + 2) % VOCAB.len()];
    format!("fn fn_{j}() {{ let {a} = compute_{b}(); use_{a}({a}); }}")
}
