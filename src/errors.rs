use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("lancedb: {0}")]
    Lance(String),

    #[error("schema migration failed: {0}")]
    Migration(String),

    #[error("vector dim mismatch: expected {expected}, got {got}")]
    VecDimMismatch { expected: usize, got: usize },

    #[error("config: {0}")]
    Config(String),

    #[error("other: {0}")]
    Other(String),
}

impl From<lancedb::Error> for Error {
    fn from(e: lancedb::Error) -> Self {
        Self::Lance(e.to_string())
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Other(format!("rusqlite: {e}"))
    }
}
