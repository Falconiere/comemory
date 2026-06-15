//! Indexing hot-path bench: `ast::extract` (extract + cAST chunk) over a
//! real comemory source file, then the per-symbol write fan-out
//! (`code_row::insert` + `fts::index_code` + `vector::insert_code`) into a
//! fresh migrated DB. Replicates the body of the private
//! `cli::index_code::write_symbol`, which only calls these public helpers.

#[path = "common/vectors.rs"]
mod vectors;

use vectors::vector;

use comemory::ast::{self, languages::Lang};
use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::{connection, fts, vector as store_vec};
use criterion::{Criterion, criterion_group, criterion_main};
use rusqlite::Connection;

/// Code vector dimension — must match the `code_vec` vec0 DDL.
const CODE_DIM: usize = 768;

/// A real comemory source file, embedded at compile time. Resolves
/// relative to this bench file (`benches/`), so `../src/...`.
const SAMPLE_SRC: &str = include_str!("../src/retrieval/pipeline.rs");

/// Open a fresh migrated `comemory.db` in a throwaway tempdir.
fn fresh_conn() -> (tempfile::TempDir, Connection) {
    let tmp = tempfile::tempdir().unwrap();
    let conn = connection::open(tmp.path().join("comemory.db")).unwrap();
    (tmp, conn)
}

/// Time `ast::extract` alone over the embedded source.
fn bench_extract(c: &mut Criterion) {
    let symbols = ast::extract(Lang::Rust, SAMPLE_SRC).unwrap();
    assert!(!symbols.is_empty(), "extract must yield symbols");
    c.bench_function("indexing/extract", |b| {
        b.iter(|| {
            let out = ast::extract(Lang::Rust, SAMPLE_SRC).unwrap();
            std::hint::black_box(out.len());
        });
    });
}

/// Time the full extract -> write fan-out into a fresh DB per iteration so
/// the inserts are not amortized across a growing table.
fn bench_index_file(c: &mut Criterion) {
    c.bench_function("indexing/index_file", |b| {
        b.iter(|| {
            let (_tmp, conn) = fresh_conn();
            let written = index_source(&conn, SAMPLE_SRC);
            assert!(written > 0, "non-zero code_symbols rows written");
            std::hint::black_box(written);
        });
    });
}

/// Extract every symbol from `src` and write its row + FTS + vector, the
/// same three public calls `cli::index_code` makes. Returns the row count.
fn index_source(conn: &Connection, src: &str) -> usize {
    let symbols = ast::extract(Lang::Rust, src).unwrap();
    for (n, sym) in symbols.iter().enumerate() {
        let path = "src/sample.rs";
        let id = code_row::insert(
            conn,
            &CodeSymbolRow {
                repo: "bench",
                path,
                blob_oid: "oid",
                symbol: &sym.name,
                kind: &sym.kind,
                lang: &sym.language,
                line_start: sym.line as i64,
                line_end: sym.line_end as i64,
                snippet: &sym.snippet,
                simhash: 0,
                parent_id: None,
            },
        )
        .unwrap();
        fts::index_code(conn, id, &sym.name, &sym.snippet, path).unwrap();
        store_vec::insert_code(conn, id, &vector(&format!("s{n}"), CODE_DIM)).unwrap();
    }
    symbols.len()
}

criterion_group!(benches, bench_extract, bench_index_file);
criterion_main!(benches);
