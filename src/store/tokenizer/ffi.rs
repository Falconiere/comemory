//! Registers the `identifier` FTS5 tokenizer on a connection via the
//! raw fts5_api. Must run before any statement that references an FTS
//! table declared with `tokenize = 'identifier'` — bundled SQLite 3.46
//! resolves tokenizers eagerly at prepare time. Registration is per
//! connection: the fts5_api tokenizer registry lives in connection
//! state, so `store::connection::open` calls [`register`] on every
//! connection it hands out, before any migration DDL runs.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use libsqlite3_sys as ffi;
use rusqlite::Connection;

use crate::prelude::*;
use crate::store::tokenizer::split::split_text;

/// Tokenizer name used in `tokenize = '...'` DDL clauses.
pub const TOKENIZER_NAME: &CStr = c"identifier";

/// Register the `identifier` tokenizer on `conn`. Safe to call more
/// than once per connection: `fts5CreateTokenizer` prepends to the
/// tokenizer list and lookup takes the first match, so a
/// re-registration shadows (not overwrites) the previous entry.
pub fn register(conn: &Connection) -> Result<()> {
    let api = fts5_api_ptr(conn)?;
    let tokenizer = ffi::fts5_tokenizer {
        xCreate: Some(x_create),
        xDelete: Some(x_delete),
        xTokenize: Some(x_tokenize),
    };
    // SAFETY: `api` was obtained from this live connection and checked
    // non-null; fts5_api v2 guarantees xCreateTokenizer is present.
    let x_create_tokenizer = unsafe { (*api).xCreateTokenizer }
        .ok_or_else(|| Error::Other("fts5_api missing xCreateTokenizer".into()))?;
    // SAFETY: `api` is valid for this call; the name is NUL-terminated
    // static; SQLite copies `tokenizer` during the call.
    let rc = unsafe {
        x_create_tokenizer(
            api,
            TOKENIZER_NAME.as_ptr(),
            ptr::null_mut(),
            &tokenizer as *const ffi::fts5_tokenizer as *mut ffi::fts5_tokenizer,
            None,
        )
    };
    if rc != ffi::SQLITE_OK {
        return Err(Error::Other(format!(
            "fts5 tokenizer registration failed: rc={rc}"
        )));
    }
    Ok(())
}

/// Fetch the connection's `fts5_api` pointer via `SELECT fts5(?1)`.
fn fts5_api_ptr(conn: &Connection) -> Result<*mut ffi::fts5_api> {
    let mut api: *mut ffi::fts5_api = ptr::null_mut();
    // SAFETY: handle() returns the live sqlite3* owned by `conn`; the
    // statement is prepared, pointer-bound, stepped and finalized within
    // this scope; "fts5_api_ptr" is the documented pointer type tag.
    unsafe {
        let db = conn.handle();
        let mut stmt: *mut ffi::sqlite3_stmt = ptr::null_mut();
        let sql = c"SELECT fts5(?1)";
        let rc = ffi::sqlite3_prepare_v2(db, sql.as_ptr(), -1, &mut stmt, ptr::null_mut());
        if rc != ffi::SQLITE_OK {
            return Err(Error::Other(format!(
                "fts5 api probe prepare failed: rc={rc}"
            )));
        }
        let rc = ffi::sqlite3_bind_pointer(
            stmt,
            1,
            &mut api as *mut *mut ffi::fts5_api as *mut c_void,
            c"fts5_api_ptr".as_ptr(),
            None,
        );
        if rc != ffi::SQLITE_OK {
            ffi::sqlite3_finalize(stmt);
            return Err(Error::Other(format!("fts5 api probe bind failed: rc={rc}")));
        }
        let rc = ffi::sqlite3_step(stmt);
        ffi::sqlite3_finalize(stmt);
        if rc != ffi::SQLITE_ROW {
            return Err(Error::Other(format!("fts5 api probe step failed: rc={rc}")));
        }
    }
    if api.is_null() {
        return Err(Error::Other(
            "FTS5 unavailable: fts5_api pointer is null".into(),
        ));
    }
    Ok(api)
}

/// FTS5 `xCreate` callback: allocate one tokenizer instance.
unsafe extern "C" fn x_create(
    _ctx: *mut c_void,
    _args: *mut *const c_char,
    _n_args: c_int,
    pp_out: *mut *mut ffi::Fts5Tokenizer,
) -> c_int {
    // Stateless tokenizer: a dangling-but-nonnull sentinel is enough.
    // SAFETY: pp_out is provided by SQLite and valid for one write.
    unsafe { *pp_out = ptr::NonNull::<ffi::Fts5Tokenizer>::dangling().as_ptr() };
    ffi::SQLITE_OK
}

/// FTS5 `xDelete` callback: free a tokenizer instance.
unsafe extern "C" fn x_delete(_t: *mut ffi::Fts5Tokenizer) {
    // Stateless: nothing to free.
}

/// Signature of the `xToken` emit callback SQLite passes to `xTokenize`.
type XToken = unsafe extern "C" fn(*mut c_void, c_int, *const c_char, c_int, c_int, c_int) -> c_int;

/// FTS5 `xTokenize` callback: split `text` and emit each token through
/// `x_token`. Must never panic — errors flow back as SQLite rc codes.
unsafe extern "C" fn x_tokenize(
    _t: *mut ffi::Fts5Tokenizer,
    ctx: *mut c_void,
    _flags: c_int,
    text: *const c_char,
    n_text: c_int,
    x_token: Option<XToken>,
) -> c_int {
    let Some(emit) = x_token else {
        return ffi::SQLITE_ERROR;
    };
    if text.is_null() || n_text < 0 {
        return ffi::SQLITE_OK;
    }
    // SAFETY: SQLite guarantees `text` points at `n_text` readable bytes
    // (not NUL-terminated, possibly invalid UTF-8 — hence lossy decode).
    let bytes = unsafe { std::slice::from_raw_parts(text.cast::<u8>(), n_text as usize) };
    let decoded = String::from_utf8_lossy(bytes);
    for tok in split_text(&decoded) {
        let flags = if tok.colocated {
            ffi::FTS5_TOKEN_COLOCATED
        } else {
            0
        };
        // Lossy decoding can shift byte offsets; clamp into range so
        // highlight() never reads out of bounds.
        let start = tok.start.min(bytes.len()) as c_int;
        let end = tok.end.min(bytes.len()) as c_int;
        // SAFETY: token text pointer/len are valid for the duration of
        // the call; SQLite copies the bytes before returning.
        let rc = unsafe {
            emit(
                ctx,
                flags,
                tok.text.as_ptr().cast::<c_char>(),
                tok.text.len() as c_int,
                start,
                end,
            )
        };
        if rc != ffi::SQLITE_OK {
            return rc;
        }
    }
    ffi::SQLITE_OK
}
