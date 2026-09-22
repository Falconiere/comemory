//! The embedding-side `comemory doctor` probes: the FTS tokenizer the
//! lexical index needs, the `vec0` dimensions the vector index was built
//! for, the embedder shell-out's own liveness, and the backlog of memories
//! stored without a usable vector.
//!
//! Split out of [`super::checks`] to keep each file inside the structure
//! gate's line ceiling. They belong together: each one answers a different
//! half of "can this engine actually answer a semantic search".

use super::checks::{Check, check, fail, ok, warn};
use crate::prelude::*;
use crate::store::Connection;

/// Check 5: the FTS5 `identifier` tokenizer (`src/store/tokenizer/`)
/// registers cleanly on `conn`. Re-registration is idempotent (see
/// `tokenizer::ffi::register`'s own doc), so this doubles as a direct
/// functional proof rather than an indirect FTS query.
pub(super) fn tokenizer(conn: &Connection) -> (Check, bool) {
    match crate::store::tokenizer::ffi::register(conn) {
        Ok(()) => (ok("fts5 tokenizer", "registered"), true),
        Err(e) => (fail("fts5 tokenizer", format!("failed: {e}")), false),
    }
}

/// Check 6: `sqlite-vec` loaded, with the `memory_vec` / `code_vec` dims
/// read from `schema_meta` rather than hardcoded.
pub(super) fn vector_dims(
    conn: &Connection,
    sqlite_vec_loaded: bool,
) -> (Check, Option<u32>, Option<u32>) {
    let memory_dim = crate::store::vector::dim_memory(conn)
        .ok()
        .and_then(|d| u32::try_from(d).ok());
    let code_dim = crate::store::vector::dim_code(conn)
        .ok()
        .and_then(|d| u32::try_from(d).ok());
    let detail = format!(
        "sqlite-vec loaded={sqlite_vec_loaded}, memory_vec dim={memory_dim:?}, \
         code_vec dim={code_dim:?}"
    );
    let result = if sqlite_vec_loaded && memory_dim.is_some() && code_dim.is_some() {
        ok("sqlite-vec", detail)
    } else {
        warn("sqlite-vec", detail)
    };
    (result, memory_dim, code_dim)
}

/// Check 8: run the configured `COMEMORY_EMBED_CMD` (`crate::utilities::embed`) and
/// time it. A missing command or a failing probe is `"warn"`, never a hard
/// error — see the module doc's "Forward-compat fallback" sibling
/// invariant: `doctor`'s job is to report a broken state, not become one.
pub(super) fn embed_probe(cmd: Option<&str>) -> (Check, Option<u64>) {
    let Some(cmd) = cmd else {
        return (warn("embed command", "COMEMORY_EMBED_CMD is not set"), None);
    };
    let started = std::time::Instant::now();
    match crate::utilities::embed::embed_query(cmd, "comemory doctor embed probe") {
        Ok(vector) => {
            let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            (
                ok(
                    "embed command",
                    format!("probe returned a {}-dim vector", vector.len()),
                ),
                Some(elapsed),
            )
        }
        Err(e) => (warn("embed command", format!("probe failed: {e}")), None),
    }
}

/// Memories whose text is stored but whose vector is not.
///
/// An import from a peer running a different embedder replicates every
/// memory correctly and still leaves this engine answering semantic search
/// short, because a vector it cannot compare against is worse than none.
/// That is a `warn`, not a `fail`: everything is present and searchable
/// lexically, and one `reembed` run fixes it.
pub(super) fn needs_embedding_check(conn: &Connection) -> Result<Check> {
    let pending = crate::store::needs_embedding::pending(conn)?.len();
    Ok(if pending == 0 {
        check(
            "embedding backlog",
            "ok",
            "every memory that should have a vector has one",
        )
    } else {
        check(
            "embedding backlog",
            "warn",
            format!(
                "{pending} {noun} stored without a usable vector; semantic search \
                 will not reach them until they are re-embedded",
                noun = if pending == 1 { "memory" } else { "memories" }
            ),
        )
        .with_remedy("POST /api/v1/doctor/reembed")
    })
}
