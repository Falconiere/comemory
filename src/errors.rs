use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("ast: {0}")]
    Ast(String),

    #[error("git: {0}")]
    Git(#[from] git2::Error),

    #[error("schema migration failed: {0}")]
    Migration(String),

    #[error("vector dim mismatch: expected {expected}, got {got}")]
    VecDimMismatch { expected: usize, got: usize },

    #[error("invalid frontmatter: {0}")]
    Frontmatter(String),

    #[error("memory not found: {0}")]
    NotFound(String),

    #[error("config: {0}")]
    Config(String),

    /// A `comemory serve` request was rejected by the security layer
    /// (token mismatch, non-loopback Host, or a path that escaped the repo
    /// root). Maps to HTTP 403; on the CLI path it maps to EX_SOFTWARE.
    #[error("forbidden: {0}")]
    Forbidden(String),

    /// A `comemory serve` request was malformed (unparsable node id,
    /// missing parameter, unknown repo root). Maps to HTTP 400; on the CLI
    /// path it maps to EX_SOFTWARE.
    #[error("bad request: {0}")]
    BadRequest(String),

    /// Required learning data is absent (no golden pairs, not enough
    /// feedback). Maps to EX_UNAVAILABLE (69).
    #[error("unavailable: {0}")]
    Unavailable(String),

    #[error("other: {0}")]
    Other(String),
}
