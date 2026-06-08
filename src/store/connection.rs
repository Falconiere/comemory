//! Opens the comemory.db SQLite connection, configures PRAGMAs, and
//! registers the sqlite-vec extension as a SQLite auto-extension so
//! every new connection inherits the `vec_*` SQL functions and virtual
//! tables.
//!
//! The v0.2 plan (Task 2.4) called for `conn.load_extension(...)`. That
//! API path requires a runtime-resolvable shared library; the
//! `sqlite-vec` crate ships the extension as a statically-linked C
//! archive via its own `build.rs`, so its `sqlite3_vec_init` entry
//! point is bound at link time, not via `dlopen`. We therefore register
//! the entry as a SQLite *auto*-extension (the same mechanism the
//! upstream `sqlite-vec` crate documents in its own integration test),
//! which is functionally equivalent: every `sqlite3_open` invokes the
//! registered list, so each rusqlite `Connection` we hand out has
//! `vec_version()` and the `vec0` virtual-table module available.

use std::path::Path;
use std::sync::OnceLock;

use rusqlite::auto_extension::{register_auto_extension, RawAutoExtension};
use rusqlite::Connection;

use crate::prelude::*;

/// Open (or create) the comemory.db file at `path` and prepare it for
/// use: WAL mode, busy_timeout=5000ms, foreign_keys=ON, sqlite-vec
/// registered as an auto-extension.
pub fn open<P: AsRef<Path>>(path: P) -> Result<Connection> {
    ensure_sqlite_vec_registered()?;
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000_i64)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(conn)
}

/// Process-wide memo of whether the sqlite-vec auto-extension has been
/// registered. SQLite's auto-extension list is global state; we register
/// once and reuse the (cached) result on every subsequent `open`.
static SQLITE_VEC_REGISTERED: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Registers `sqlite3_vec_init` from the statically-linked sqlite-vec
/// crate as a SQLite auto-extension exactly once per process.
fn ensure_sqlite_vec_registered() -> Result<()> {
    let outcome = SQLITE_VEC_REGISTERED.get_or_init(register_sqlite_vec);
    match outcome {
        Ok(()) => Ok(()),
        Err(msg) => Err(Error::Other(msg.clone())),
    }
}

/// One-shot registration body, invoked by `OnceLock::get_or_init`.
///
/// Reinterprets sqlite-vec's `unsafe extern "C" fn()` entry as the
/// `RawAutoExtension` signature SQLite expects (same C calling
/// convention, the wider arg list is what SQLite actually passes —
/// matches the upstream `sqlite-vec` crate's documented integration).
/// `register_auto_extension` mutates SQLite's process-global list and is
/// itself `unsafe`; `OnceLock` ensures we run this body exactly once.
fn register_sqlite_vec() -> std::result::Result<(), String> {
    let entry: unsafe extern "C" fn() = sqlite_vec::sqlite3_vec_init;
    // SAFETY: see this fn's doc comment — ABI-compatible fn-ptr
    // transmute, one-shot global registration via `OnceLock`.
    unsafe {
        let raw: RawAutoExtension = std::mem::transmute(entry);
        register_auto_extension(raw).map_err(|e| format!("register sqlite-vec: {e}"))
    }
}
