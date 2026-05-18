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
        self.root.path().join(".qwick")
    }
}
