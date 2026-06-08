//! Opens the comemory.db SQLite connection, configures PRAGMAs, and
//! registers the sqlite-vec extension as a SQLite auto-extension so
//! every new connection inherits the `vec_*` SQL functions and virtual
//! tables.

use std::path::Path;
use std::sync::Once;

use rusqlite::Connection;

use crate::prelude::*;

/// Open (or create) the comemory.db file at `path` and prepare it for
/// use: WAL mode, busy_timeout=5000ms, foreign_keys=ON, sqlite-vec
/// registered as an auto-extension.
pub fn open<P: AsRef<Path>>(path: P) -> Result<Connection> {
    register_sqlite_vec()?;
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000_i64)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(conn)
}

static REGISTER_VEC: Once = Once::new();
static mut REGISTER_VEC_RESULT: std::result::Result<(), String> = Ok(());

/// Registers `sqlite3_vec_init` from the statically-linked sqlite-vec
/// crate as a SQLite auto-extension exactly once per process.
fn register_sqlite_vec() -> Result<()> {
    REGISTER_VEC.call_once(register_once);
    read_register_result()
}

/// One-shot body of [`register_sqlite_vec`].
fn register_once() {
    let entry: unsafe extern "C" fn() = sqlite_vec::sqlite3_vec_init;
    // SAFETY: transmute reinterprets the sqlite-vec extension entry as
    // SQLite's RawAutoExtension C ABI (matching how SQLite invokes
    // every loadable extension); `Once::call_once` serializes the write.
    unsafe {
        let raw: rusqlite::auto_extension::RawAutoExtension = std::mem::transmute(entry);
        if let Err(e) = rusqlite::auto_extension::register_auto_extension(raw) {
            REGISTER_VEC_RESULT = Err(format!("register sqlite-vec: {e}"));
        }
    }
}

/// Reads the one-shot registration result after the `Once` has fired.
fn read_register_result() -> Result<()> {
    let addr = std::ptr::addr_of!(REGISTER_VEC_RESULT);
    // SAFETY: `REGISTER_VEC.call_once` happens-before this load, so the
    // single write to `REGISTER_VEC_RESULT` is fully visible and no
    // concurrent writer remains.
    let result_ref = unsafe { &*addr };
    match result_ref {
        Ok(()) => Ok(()),
        Err(msg) => Err(Error::Other(msg.clone())),
    }
}
