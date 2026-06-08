use std::path::{Path, PathBuf};

use crate::prelude::*;

#[derive(Debug, Clone)]
pub struct Paths {
    data_dir: PathBuf,
}

impl Paths {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn memories_dir(&self) -> PathBuf {
        self.data_dir.join("memories")
    }

    pub fn trash_dir(&self) -> PathBuf {
        self.memories_dir().join(".trash")
    }

    pub fn index_dir(&self) -> PathBuf {
        self.data_dir.join("index")
    }

    pub fn vectors_dir(&self) -> PathBuf {
        self.index_dir().join("vectors.lance")
    }

    pub fn graph_dir(&self) -> PathBuf {
        self.index_dir().join("graph.kuzu")
    }

    /// Path to the SQLite FTS5 BM25 index used for lexical memory search.
    pub fn fts_db(&self) -> PathBuf {
        self.index_dir().join("fts.sqlite")
    }

    pub fn stats_db(&self) -> PathBuf {
        self.data_dir.join("stats.db")
    }

    /// Single-file SQLite mirror for v0.2 (`comemory.db`). Rooted directly at
    /// the data dir so callers that override `COMEMORY_DATA_DIR` see a
    /// predictable path next to `memories/`.
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("comemory.db")
    }

    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.toml")
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [self.memories_dir(), self.trash_dir(), self.index_dir()] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}
