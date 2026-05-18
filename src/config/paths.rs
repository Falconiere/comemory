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

    pub fn stats_db(&self) -> PathBuf {
        self.data_dir.join("stats.db")
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
