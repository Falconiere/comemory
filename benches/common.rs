//! Shared bench harness: deterministic temp data dir + a seeded corpus so
//! every bench compares apples to apples.

use std::path::PathBuf;

use comemory::config::paths::Paths;
use comemory::index::{Embedder, Fts, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};
use tempfile::TempDir;

pub struct Fixture {
    pub _tmp: TempDir,
    pub paths: Paths,
}

pub fn fixture() -> Fixture {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::new(tmp.path().join(".comemory"));
    paths.ensure_dirs().expect("ensure_dirs");
    Fixture { _tmp: tmp, paths }
}

#[allow(dead_code)]
pub async fn seed(paths: &Paths, n: usize) -> Vec<String> {
    let store = MemoryStore::new(paths.clone());
    let mut emb = Embedder::nomic_text().expect("embedder");
    let idx = MemoryIndex::open(paths.vectors_dir(), 768)
        .await
        .expect("memory index");
    let fts = Fts::open(paths.fts_db()).expect("fts");
    let bodies: Vec<String> = (0..n)
        .map(|i| format!("seed body {i}: postgres analytics token_{i}"))
        .collect();
    let mut ids = Vec::with_capacity(n);
    for body in &bodies {
        let rec = store
            .save(body, Kind::Note, "bench", &[], "bench", 3)
            .expect("save");
        let v = emb.embed_one(&rec.body).expect("embed");
        idx.upsert(&rec, &v).await.expect("upsert");
        fts.upsert(&rec.frontmatter.id, &rec.body)
            .expect("fts upsert");
        ids.push(rec.frontmatter.id);
    }
    ids
}

#[allow(dead_code)]
pub fn data_dir(f: &Fixture) -> PathBuf {
    f.paths.data_dir().to_path_buf()
}
