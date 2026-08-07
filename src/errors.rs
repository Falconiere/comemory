use thiserror::Error;

/// Crate-wide `Result` alias, defaulting the error type to [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The crate-wide error enum every fallible operation returns through.
#[derive(Debug, Error)]
pub enum Error {
    /// An underlying filesystem operation failed (read, write, rename).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// A SQLite operation failed (query, migration, or `sqlite-vec`/FTS5
    /// extension call).
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Frontmatter YAML failed to parse or serialize.
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),

    /// A JSON payload failed to parse or serialize (CLI `--json` output,
    /// `--vector-stdin`, embed-command replies).
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// A TOML config file failed to parse.
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),

    /// The AST extractor (ast-grep) could not process a source file.
    #[error("ast: {0}")]
    Ast(String),

    /// A `git2` repository operation failed (open, blob lookup, hook
    /// install).
    #[error("git: {0}")]
    Git(#[from] git2::Error),

    /// A versioned SQLite schema migration failed to apply.
    #[error("schema migration failed: {0}")]
    Migration(String),

    /// A supplied vector's dimensionality does not match the `vec0` column
    /// baked into the schema at migration time.
    #[error("vector dim mismatch: expected {expected}, got {got}")]
    VecDimMismatch {
        /// The dimensionality the `vec0` column was created with.
        expected: usize,
        /// The dimensionality of the vector actually supplied.
        got: usize,
    },

    /// A memory's YAML frontmatter failed schema validation.
    #[error("invalid frontmatter: {0}")]
    Frontmatter(String),

    /// A document (TXT/Markdown/HTML/CSV) failed to extract — malformed
    /// input the in-process extractor could not parse.
    #[error("document extract: {0}")]
    Document(String),

    /// The requested memory id has no matching row / markdown file.
    #[error("memory not found: {0}")]
    NotFound(String),

    /// A command-line invocation was malformed (a bad flag value, an empty or
    /// ill-formed argument). Maps to `EX_USAGE` (exit 64) with a plain
    /// `error:` prefix — distinct from [`Error::NotFound`]'s "memory not
    /// found" wording.
    #[error("{0}")]
    Usage(String),

    /// Layered configuration (defaults / file / env) failed validation.
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

    /// A `comemory serve` confirm-gated mutating route (e.g. `DELETE
    /// /memories/{id}`, `POST /rebuild`) was called without the required
    /// `"confirm":true` body field / `?confirm=true` query param. Maps to
    /// HTTP 400 `code:"confirmation_required"`; on the CLI path it maps to
    /// EX_SOFTWARE. Distinct from [`Error::BadRequest`] so
    /// `serve::envelope::status_and_code` can give it its own `code` slug
    /// without sniffing message text.
    #[error("confirmation required: {0}")]
    ConfirmationRequired(String),

    /// Required learning data is absent (no golden pairs, not enough
    /// feedback). Maps to EX_UNAVAILABLE (69).
    #[error("unavailable: {0}")]
    Unavailable(String),

    /// A catch-all for failures that don't fit another variant.
    #[error("other: {0}")]
    Other(String),
}
