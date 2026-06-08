//! Build script for comemory v0.2.
//!
//! The `sqlite-vec` extension is compiled and statically linked by the
//! `sqlite-vec` crate's own `build.rs` (uses `cc` to compile
//! `sqlite-vec.c` into `libsqlite_vec0.a`). Our binary picks the
//! resulting archive up via the `#[link(name = "sqlite_vec0")]` attribute
//! on `sqlite_vec::sqlite3_vec_init` — see `src/store/connection.rs`,
//! which registers that symbol as a SQLite auto-extension so every
//! freshly opened connection gets `vec_*` SQL functions and the `vec0`
//! virtual-table module.
//!
//! Task 2 of the v0.2 plan called for this top-level `build.rs` to
//! compile the vendored C source itself with `cc`. That is intentionally
//! delegated to the upstream crate to avoid a second copy of the
//! compilation step: the link result is identical, but the build graph
//! stays single-rooted. `cc` is still declared in `[build-dependencies]`
//! so this file remains the documented home for any future vendored
//! C-source work.
//!
//! Keeping this script in place also makes it trivial to drop in custom
//! `cargo:rerun-if-changed` directives or compile additional vendored
//! sources without rewiring `Cargo.toml`.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
}
