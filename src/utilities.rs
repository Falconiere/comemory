//! Transport-neutral shared primitives (#166).
//!
//! Everything here is usable by a domain, by `cli`, and by `serve` alike:
//! nothing in this module may depend on `cli`, `serve`, or `output`. Each
//! file owns exactly one primitive, named after it, so a caller reaches the
//! concern directly instead of through a technical-layer barrel.
//!
//! Four helpers still name a capability-owned type whose owner is deliberately
//! not moving here: `id_list` delegates memory-id *policy* to
//! `memory::id::is_valid_memory_id`, `when` returns `retrieval::scope::TimeScope`
//! (and formats bounds through `store::memory_row`), and `ref_args` builds
//! `memory::References` from `git_utils` lookups. Those owners move under
//! `domains::` in #167/#169/#171; the direction (shared → capability) stays
//! legal for a shared utility under the final tree.
//!
//! `src/lib.rs` re-exports [`embed`], [`fetch`], [`http_error`] and
//! [`simhash`] at the crate root: those four were public root modules before
//! the move, and the aliases keep `comemory::<name>` resolving for external
//! consumers (Binding Rule 1 — one implementation, one home).

/// The transport-neutral execution context every command core runs against.
pub mod context;
/// SHA-256 hex digests and the lowercase-hex shape check built on them.
pub(crate) mod digest;
/// Shared embed-command shell-out (`COMEMORY_EMBED_CMD`).
pub mod embed;
/// Pure decoding of `--vector` CSV and `{"embedding":[..]}` JSON payloads.
pub(crate) mod embedding_input;
/// Shell out to `curl` (falling back to `wget`) for HTTP.
pub mod fetch;
/// Exclusive advisory lock over a sibling lock file.
pub mod file_lock;
/// Map outbound `reqwest` transport errors into [`crate::Error`].
pub mod http_error;
/// Comma-separated id-list splitting, de-duplication, and validation.
pub(crate) mod id_list;
/// Page windows, the generic page envelope, and the retrieval page cursor.
pub mod pagination;
/// Canonicalize-and-contain path checks shared by every filesystem surface.
pub mod path_containment;
/// The progress / cancellation contract long-running jobs report through.
pub mod progress;
/// The `q-<yyyymmdd>-<8hex>` retrieval-log query id: mint and validate.
pub mod query_id;
/// Collect the `--ref-file` / `--ref-symbol` values into a `References` block.
pub mod ref_args;
/// 64-bit SimHash and Hamming distance over tokenized bodies.
pub mod simhash;
/// The persisted retrieval-log / feedback vocabularies.
pub(crate) mod telemetry;
/// Acquire a caller-supplied vector from the flags and process stdin.
pub(crate) mod vector_stdin;
/// `--since` / `--until` / `--as-of` value parsing.
pub mod when;
