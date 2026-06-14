//! Shared helpers for `tests/cli__ingest_code.rs` and
//! `tests/cli__ingest_code_2.rs`.
//!
//! Both including binaries also declare a sibling `mod vectors;` (the
//! deterministic-vector helper), which this module reaches through
//! `crate::vectors` rather than re-`#[path]`-including the file — a second
//! include would trip rustc's "file loaded as a module multiple times".

/// Build a minimal valid JSONL row string. `seed` drives the embedding so
/// every call produces a distinct (but deterministic) vector.
pub fn make_row(seed: &str, repo: &str, path: &str, blob_oid: &str) -> String {
    let embedding = crate::vectors::vector(seed, 768);
    serde_json::to_string(&serde_json::json!({
        "repo": repo,
        "path": path,
        "blob_oid": blob_oid,
        "symbol": format!("sym_{seed}"),
        "kind": "function",
        "lang": "rust",
        "line_start": 1_u32,
        "line_end": 3_u32,
        "snippet": format!("fn {seed}() {{}}"),
        "simhash": 0_i64,
        "embedding": embedding,
    }))
    .expect("row json")
}
