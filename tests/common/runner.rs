#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
use std::path::PathBuf;
use tempfile::TempDir;

pub struct Sandbox {
    pub root: TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        Self {
            root: TempDir::new().unwrap(),
        }
    }

    pub fn data_dir(&self) -> PathBuf {
        self.root.path().join(".comemory")
    }
}
