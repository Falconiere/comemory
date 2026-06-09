# M1 Rank-Blend Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Recall-quality upgrade for comemory: identifier-aware FTS5 tokenizer, BM25 column weights, ACT-R activation + Beta-feedback + quality priors in a two-stage retrieve→rerank→diversify pipeline, save-time duplicate warning, and prune rewired to usage signals.

**Architecture:** A custom FTS5 tokenizer (`identifier`) registered per-connection via `libsqlite3-sys` FFI splits camelCase/snake_case/digit identifiers; migration 0004 rebuilds both FTS tables with it and adds `access_count`/`last_accessed`/`simhash` columns. Retrieval becomes `router (top-50 candidates) → rerank (multiplicative bounded priors) → diversify (SimHash collapse + MMR) → top-k`, with `score_parts` in JSON output. Prune low-value detection consumes the same activation/feedback signals.

**Tech Stack:** Rust, rusqlite 0.32 (bundled SQLite 3.46, FTS5), libsqlite3-sys 0.30 FFI, time 0.3, proptest, insta, assert_cmd, cargo-nextest.

**Spec:** `docs/superpowers/specs/2026-06-09-m1-rank-blend-core-design.md`. Two corrections discovered during planning: the schema is already at version 3 (`0003_stats_tables.sql` ships today), so the new migration is **`0004_v4_rank.sql` / version "4"**, not 0003/v3 as the spec says. And `memories` has no `simhash` column today — 0004 adds one (needed for the save-time dup check and query-time collapse).

**Binding rules reminder (enforced by `scripts/check-all.sh`):** no `.unwrap()` in `src/`, no `#[cfg(test)] mod tests` in `src/`, tests mirror `src/` 1:1 under `tests/`, ≤500 lines per file, `unsafe` only with a `// SAFETY:` comment within 3 lines above, doc comments on public items, no `println!`/`panic!` in `src/`. After creating any new `src/` file, run `bash scripts/tests-mirror-check.sh` and add the mirror test file/module declarations it expects.

**Existing API surface you will touch (verified):**

- `src/store/connection.rs:29` `pub fn open<P: AsRef<Path>>(path: P) -> Result<Connection>` — WAL, busy_timeout, sqlite-vec autoload, then `migrate::run`.
- `src/store/migrate.rs:16` `pub const CURRENT_VERSION: &str = "3"`, consts `M_BOOTSTRAP`/`M_V2`/`M_V3` via `include_str!`, `pub fn run(conn: &mut Connection)`, `fn apply(conn, key, sql) -> Result<()>`.
- `memory_fts` DDL today: `fts5(memory_id UNINDEXED, body, tags, tokenize = 'porter unicode61 remove_diacritics 2')`; `code_fts`: `fts5(symbol_id UNINDEXED, symbol, snippet, path_tokens, tokenize = 'unicode61 remove_diacritics 2')`.
- `src/store/fts.rs:40` `pub fn search_memory(conn, query, k, repo) -> Result<Vec<MemoryFtsHit>>` (BM25 ASC, FTS5 parse errors → empty vec).
- `src/retrieval/router.rs:52` `pub fn route(cfg: &Config, conn: &Connection, query: &str, vec: Option<&[f32]>, repo: Option<&str>) -> Result<Vec<RoutedHit>>`; `RoutedHit { memory_id: String, score: f32, source: Source }`, `enum Source { Vector, Lexical, Hybrid }`.
- `src/retrieval/fuse.rs:34` `pub fn rrf_k(a: &[RankedHit], b: &[RankedHit], top_k: usize, k: f32) -> Vec<RankedHit>`.
- `feedback` table: `(memory_id TEXT PRIMARY KEY, used_count INTEGER, irrelevant_count INTEGER, last_used TEXT)` — lives in comemory.db.
- `edges` table: `(src_kind, src_id, dst_kind, dst_id, rel, created_at)`, `rel` includes `'supersedes'`.
- `src/simhash.rs`: `pub fn simhash64<I, T>(tokens: I) -> u64`, `pub fn hamming64(a, b) -> u32`, `pub fn tokens(snippet: &str) -> Vec<String>`.
- `src/config/file.rs`: `Config { git, embeddings, indexing, retrieval, prune, output, embed_hint }`, `defaults()`, `with_file()`, `with_env()`; env wiring pattern at lines 188-251.
- `src/cli/search.rs:62` `pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>)` → `router::route` → `output::search::emit(hits, json_flag)`.
- `src/cli/save.rs:85` run() → `MemoryStore::save` (atomic markdown) → `write_sqlite_mirror` (tx: `memory_row::insert` + optional vector).
- `src/prune/low_value.rs:26` `pub fn detect(paths: &Paths, below_quality: u8, unused_since_days: u32) -> Result<Vec<String>>`.
- `src/cli/prune.rs`: `Args { dry_run: bool }`, `Report { orphan_edges: i64, stale_code_files: Vec<String> }`, `fn scan`, `fn apply`.
- Tests: `tests/<module>.rs` thin shims declaring `tests/<module>/` submodules; `tests/common/runner.rs` has `Sandbox::new()` + `data_dir()`; assert_cmd pattern in `tests/cli.rs`; insta in `tests/output.rs`.
- Errors: `crate::prelude::{Error, Result}`; add variants to `src/errors.rs` if needed (`Error::Other(String)` exists).

---

### Task 1: Identifier splitting logic (pure Rust, no FFI)

**Files:**
- Create: `src/store/tokenizer/mod.rs`
- Create: `src/store/tokenizer/split.rs`
- Modify: `src/store/mod.rs` (add `pub(crate) mod tokenizer;` — match existing module declaration style)
- Test: `tests/store/tokenizer_split.rs` (+ declare in `tests/store.rs`: `mod tokenizer_split;` under the existing submodule path `tests/store/`)

- [ ] **Step 1: Write the failing tests**

`tests/store/tokenizer_split.rs`:

```rust
use comemory::store::tokenizer::split::{split_text, SplitToken};
use proptest::prelude::*;

fn texts(tokens: &[SplitToken]) -> Vec<&str> {
    tokens.iter().map(|t| t.text.as_str()).collect()
}

#[test]
fn camel_case_splits_with_colocated_whole() {
    let toks = split_text("VecDimMismatch");
    // parts at distinct positions, whole identifier colocated with first part
    assert_eq!(texts(&toks), vec!["vec", "vecdimmismatch", "dim", "mismatch"]);
    assert!(!toks[0].colocated);
    assert!(toks[1].colocated);
    assert!(!toks[2].colocated);
    assert!(!toks[3].colocated);
}

#[test]
fn snake_case_splits() {
    let toks = split_text("parse_html");
    assert_eq!(texts(&toks), vec!["parse", "parse_html", "html"]);
}

#[test]
fn digit_boundaries_split() {
    let toks = split_text("sha256sum");
    assert_eq!(texts(&toks), vec!["sha", "sha256sum", "256", "sum"]);
}

#[test]
fn plain_words_emit_single_token() {
    let toks = split_text("hello world");
    assert_eq!(texts(&toks), vec!["hello", "world"]);
    assert!(toks.iter().all(|t| !t.colocated));
}

#[test]
fn byte_offsets_point_into_original_text() {
    let text = "use VecDim now";
    for t in split_text(text) {
        if t.text == "vecdim" || t.text == "vec" || t.text == "dim" {
            // offsets must cover the run "VecDim" (bytes 4..10) or a sub-range
            assert!(t.start >= 4 && t.end <= 10, "bad offsets {}..{}", t.start, t.end);
        }
    }
}

#[test]
fn acronym_runs_stay_grouped() {
    // HTMLParser → html + parser (uppercase run followed by lowercase keeps last cap with next part)
    let toks = split_text("HTMLParser");
    assert_eq!(texts(&toks), vec!["html", "htmlparser", "parser"]);
}

proptest! {
    #[test]
    fn never_panics_and_tokens_are_lowercase(s in "\\PC*") {
        for t in split_text(&s) {
            prop_assert!(t.text.chars().all(|c| !c.is_uppercase()));
            prop_assert!(t.end <= s.len());
            prop_assert!(t.start <= t.end);
        }
    }

    #[test]
    fn first_token_of_a_run_is_never_colocated(s in "\\PC*") {
        let toks = split_text(&s);
        if let Some(first) = toks.first() {
            prop_assert!(!first.colocated);
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --all-features -E 'binary(store)'`
Expected: compile FAILURE — `tokenizer` module not found.

- [ ] **Step 3: Implement the splitter**

`src/store/tokenizer/mod.rs`:

```rust
//! Identifier-aware FTS5 tokenizer: pure splitting logic plus the FFI
//! registration that exposes it to SQLite as `tokenize = 'identifier'`.

pub(crate) mod split;
```

(`ffi` submodule is added in Task 2.)

`src/store/tokenizer/split.rs`:

```rust
//! Splits text into FTS tokens with camelCase / snake_case / digit
//! boundary awareness. Each alphanumeric run yields its sub-parts at
//! distinct token positions; when a run splits into more than one part,
//! the whole lowercased run is also emitted colocated with the first
//! part so exact-identifier queries still match.

/// One token produced by [`split_text`]: lowercased text, byte range in
/// the original input, and whether FTS5 should treat it as colocated
/// with the previous token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SplitToken {
    pub(crate) text: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) colocated: bool,
}

/// Character classes that delimit sub-parts inside an identifier run.
#[derive(Clone, Copy, PartialEq)]
enum Class {
    Lower,
    Upper,
    Digit,
    Other,
}

fn classify(c: char) -> Class {
    if c.is_lowercase() {
        Class::Lower
    } else if c.is_uppercase() {
        Class::Upper
    } else if c.is_ascii_digit() {
        Class::Digit
    } else {
        Class::Other
    }
}

/// Tokenize `text`. Runs are maximal sequences of alphanumeric chars and
/// `_`. Sub-part boundaries: `_`, lower→upper transitions, letter↔digit
/// transitions, and upper→lower transitions that end an uppercase run
/// (`HTMLParser` → `html` + `parser`).
pub(crate) fn split_text(text: &str) -> Vec<SplitToken> {
    let mut out = Vec::new();
    let mut run_start: Option<usize> = None;
    let mut iter = text.char_indices().peekable();
    while let Some(&(i, c)) = iter.peek() {
        let is_word = c.is_alphanumeric() || c == '_';
        match (run_start, is_word) {
            (None, true) => {
                run_start = Some(i);
                iter.next();
            }
            (Some(s), false) => {
                emit_run(text, s, i, &mut out);
                run_start = None;
                iter.next();
            }
            _ => {
                iter.next();
            }
        }
    }
    if let Some(s) = run_start {
        emit_run(text, s, text.len(), &mut out);
    }
    out
}

/// Emit tokens for one run `text[s..e]`.
fn emit_run(text: &str, s: usize, e: usize, out: &mut Vec<SplitToken>) {
    let run = &text[s..e];
    let parts = part_ranges(run);
    let whole = run.to_lowercase();
    if parts.len() <= 1 {
        if !whole.is_empty() && whole != "_" {
            out.push(SplitToken { text: whole, start: s, end: e, colocated: false });
        }
        return;
    }
    let mut first = true;
    for (ps, pe) in parts {
        let part = run[ps..pe].to_lowercase();
        if part.is_empty() {
            continue;
        }
        out.push(SplitToken {
            text: part,
            start: s + ps,
            end: s + pe,
            colocated: false,
        });
        if first {
            // whole identifier rides along at the first part's position
            out.push(SplitToken { text: whole.clone(), start: s, end: e, colocated: true });
            first = false;
        }
    }
}

/// Byte ranges of sub-parts inside a run, splitting at `_` and at class
/// transitions. An upper→lower transition splits *before* the last
/// uppercase char (`HTMLParser` → `HTML` would otherwise swallow the P).
fn part_ranges(run: &str) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = run.char_indices().collect();
    let mut ranges = Vec::new();
    let mut start: Option<usize> = None;
    for w in 0..chars.len() {
        let (i, c) = chars[w];
        if c == '_' {
            if let Some(s) = start.take() {
                ranges.push((s, i));
            }
            continue;
        }
        let cls = classify(c);
        if let Some(s) = start {
            let prev = classify(chars[w - 1].1);
            let lower_to_upper = prev == Class::Lower && cls == Class::Upper;
            let letter_digit = (prev == Class::Digit) != (cls == Class::Digit);
            let upper_run_ends = prev == Class::Upper
                && cls == Class::Lower
                && w >= 2
                && classify(chars[w - 2].1) == Class::Upper;
            if lower_to_upper || letter_digit {
                ranges.push((s, i));
                start = Some(i);
            } else if upper_run_ends {
                // split before the previous (uppercase) char
                let prev_i = chars[w - 1].0;
                if prev_i > s {
                    ranges.push((s, prev_i));
                    start = Some(prev_i);
                }
            }
        } else {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        ranges.push((s, run.len()));
    }
    ranges
}
```

Export path: in `src/store/mod.rs`, declare the module the same way the sibling modules are declared (`pub mod tokenizer;` if siblings are `pub`, otherwise `pub(crate)` — but note the test accesses `comemory::store::tokenizer::split::split_text`, so `split_text`/`SplitToken` and the module chain must be reachable from the test binary: use `pub mod tokenizer;` + `pub mod split;` + `pub fn split_text` / `pub struct SplitToken` if `pub(crate)` breaks the integration-test import. Tests live in `tests/` (separate crate), so **public visibility is required** — make `tokenizer`, `split`, `split_text`, and `SplitToken` `pub`, with doc comments).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --all-features -E 'binary(store)'`
Expected: PASS (all new tokenizer_split tests green).

- [ ] **Step 5: Mirror check + commit**

Run: `bash scripts/tests-mirror-check.sh && bash scripts/test-placement-check.sh`
Expected: exit 0. If the mirror check demands a different test path for `src/store/tokenizer/split.rs` (e.g. `tests/store/tokenizer/split.rs`), move the test file to the demanded path and declare it accordingly.

```bash
git add src/store/tokenizer src/store/mod.rs tests/store
git commit -m "feat(store): identifier-aware token splitting for FTS5 tokenizer"
```

---

### Task 2: FTS5 tokenizer FFI registration

**Files:**
- Create: `src/store/tokenizer/ffi.rs`
- Modify: `src/store/tokenizer/mod.rs` (add `pub mod ffi;`)
- Modify: `src/store/connection.rs` (register tokenizer in `open()` **before** `migrate::run`)
- Modify: `Cargo.toml` (add `libsqlite3-sys = "0.30"` as a direct dependency — it is already in the lock via rusqlite `bundled`)
- Test: `tests/store/tokenizer_ffi.rs` (declare in `tests/store.rs`)

- [ ] **Step 1: Write the failing test**

`tests/store/tokenizer_ffi.rs`:

```rust
use rusqlite::Connection;

fn conn_with_tokenizer() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    comemory::store::tokenizer::ffi::register(&conn).expect("register tokenizer");
    conn
}

#[test]
fn identifier_tokenizer_creates_table_and_matches_subtokens() {
    let conn = conn_with_tokenizer();
    conn.execute_batch(
        "CREATE VIRTUAL TABLE t USING fts5(body, tokenize = 'identifier');
         INSERT INTO t(body) VALUES ('returns VecDimMismatch on bad embedder');",
    )
    .expect("create + insert");

    // subtoken match
    let n: i64 = conn
        .query_row("SELECT count(*) FROM t WHERE t MATCH 'mismatch'", [], |r| r.get(0))
        .expect("query");
    assert_eq!(n, 1);

    // whole-identifier match (colocated token)
    let n: i64 = conn
        .query_row("SELECT count(*) FROM t WHERE t MATCH 'vecdimmismatch'", [], |r| r.get(0))
        .expect("query");
    assert_eq!(n, 1);

    // query-side splitting: camelCase query finds prose doc
    conn.execute("INSERT INTO t(body) VALUES ('the dim mismatch guard fires')", [])
        .expect("insert");
    let n: i64 = conn
        .query_row("SELECT count(*) FROM t WHERE t MATCH 'DimMismatch'", [], |r| r.get(0))
        .expect("query");
    assert_eq!(n, 2);
}

#[test]
fn porter_wraps_identifier() {
    let conn = conn_with_tokenizer();
    conn.execute_batch(
        "CREATE VIRTUAL TABLE t2 USING fts5(body, tokenize = 'porter identifier');
         INSERT INTO t2(body) VALUES ('indexing the repository');",
    )
    .expect("create + insert");
    let n: i64 = conn
        .query_row("SELECT count(*) FROM t2 WHERE t2 MATCH 'indexed'", [], |r| r.get(0))
        .expect("query");
    assert_eq!(n, 1);
}

#[test]
fn store_open_registers_tokenizer() {
    // store::connection::open must work end-to-end (registration precedes migration)
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let conn = comemory::store::connection::open(&db).expect("open store");
    // memory_fts is created by migration 0004 with tokenize='porter identifier'
    let n: i64 = conn
        .query_row("SELECT count(*) FROM memory_fts", [], |r| r.get(0))
        .expect("memory_fts exists and is queryable");
    assert_eq!(n, 0);
}
```

(The third test will only fully pass after Task 3 ships migration 0004; at this task's end it must pass against the *current* schema too because registration alone doesn't change DDL — `memory_fts` exists already with the old tokenizer. Keep the test as written; it is schema-agnostic.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --all-features -E 'binary(store)'`
Expected: compile FAILURE — `ffi` module not found.

- [ ] **Step 3: Implement FFI registration**

`Cargo.toml` `[dependencies]` (version must match the one rusqlite 0.32 locks, check `cargo tree -p libsqlite3-sys` — expected `0.30`):

```toml
libsqlite3-sys = "0.30"
```

`src/store/tokenizer/ffi.rs` (complete file):

```rust
//! Registers the `identifier` FTS5 tokenizer on a connection via the
//! raw fts5_api. Must run before any statement that references an FTS
//! table declared with `tokenize = 'identifier'` — bundled SQLite 3.46
//! resolves tokenizers eagerly at prepare time.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::ptr;

use libsqlite3_sys as ffi;
use rusqlite::Connection;

use crate::prelude::*;
use crate::store::tokenizer::split::split_text;

/// Tokenizer name used in `tokenize = '...'` DDL clauses.
pub const TOKENIZER_NAME: &CStr = c"identifier";

/// Register the `identifier` tokenizer on `conn`. Idempotent per
/// connection (re-registration overwrites the same entry).
pub fn register(conn: &Connection) -> Result<()> {
    let api = fts5_api_ptr(conn)?;
    let tokenizer = ffi::fts5_tokenizer {
        xCreate: Some(x_create),
        xDelete: Some(x_delete),
        xTokenize: Some(x_tokenize),
    };
    let x_create_tokenizer =
        // SAFETY: `api` was just obtained from this live connection and
        // checked non-null; fts5_api v2 guarantees xCreateTokenizer.
        unsafe { (*api).xCreateTokenizer }
            .ok_or_else(|| Error::Other("fts5_api missing xCreateTokenizer".into()))?;
    // SAFETY: `api` is valid for the duration of this call; the name is a
    // NUL-terminated static; SQLite copies `tokenizer` during the call.
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
        return Err(Error::Other(format!("fts5 tokenizer registration failed: rc={rc}")));
    }
    Ok(())
}

/// Fetch the connection's `fts5_api` pointer via `SELECT fts5(?1)`.
fn fts5_api_ptr(conn: &Connection) -> Result<*mut ffi::fts5_api> {
    let mut api: *mut ffi::fts5_api = ptr::null_mut();
    // SAFETY: handle() returns the live sqlite3* owned by `conn`; stmt is
    // prepared, pointer-bound, stepped and finalized within this scope.
    unsafe {
        let db = conn.handle();
        let mut stmt: *mut ffi::sqlite3_stmt = ptr::null_mut();
        let sql = c"SELECT fts5(?1)";
        let rc = ffi::sqlite3_prepare_v2(db, sql.as_ptr(), -1, &mut stmt, ptr::null_mut());
        if rc != ffi::SQLITE_OK {
            return Err(Error::Other(format!("fts5 api probe prepare failed: rc={rc}")));
        }
        ffi::sqlite3_bind_pointer(
            stmt,
            1,
            &mut api as *mut *mut ffi::fts5_api as *mut c_void,
            c"fts5_api_ptr".as_ptr(),
            None,
        );
        ffi::sqlite3_step(stmt);
        ffi::sqlite3_finalize(stmt);
    }
    if api.is_null() {
        return Err(Error::Other("FTS5 unavailable: fts5_api pointer is null".into()));
    }
    Ok(api)
}

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

unsafe extern "C" fn x_delete(_t: *mut ffi::Fts5Tokenizer) {
    // Stateless: nothing to free.
}

type XToken = unsafe extern "C" fn(
    *mut c_void,
    c_int,
    *const c_char,
    c_int,
    c_int,
    c_int,
) -> c_int;

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
        let flags = if tok.colocated { ffi::FTS5_TOKEN_COLOCATED } else { 0 };
        // Lossy decoding can shift byte offsets; clamp into range so
        // highlight() never reads out of bounds.
        let start = tok.start.min(bytes.len()) as c_int;
        let end = tok.end.min(bytes.len()) as c_int;
        // SAFETY: token text pointer/len are valid for the call; SQLite
        // copies the bytes before returning.
        let rc = unsafe {
            emit(ctx, flags, tok.text.as_ptr().cast::<c_char>(), tok.text.len() as c_int, start, end)
        };
        if rc != ffi::SQLITE_OK {
            return rc;
        }
    }
    ffi::SQLITE_OK
}
```

`src/store/connection.rs` — inside `open()`, **after** the sqlite-vec registration + PRAGMA setup and **before** `migrate::run(&mut conn)`:

```rust
crate::store::tokenizer::ffi::register(&conn)?;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --all-features -E 'binary(store)'`
Expected: PASS.

- [ ] **Step 5: Gate scripts + commit**

Run: `bash scripts/no-bypass-check.sh && bash scripts/tests-mirror-check.sh && cargo clippy --all-targets --all-features -- -D warnings` — the hook may forbid direct clippy; if so run `bash scripts/lint-check.sh`.
Expected: exit 0 (all `unsafe` blocks carry `// SAFETY:` within 3 lines).

```bash
git add src/store/tokenizer Cargo.toml Cargo.lock src/store/connection.rs tests/store
git commit -m "feat(store): register custom identifier FTS5 tokenizer per connection"
```

---

### Task 3: Migration 0004 — access columns, simhash, FTS rebuild

**Files:**
- Create: `src/store/sql/0004_v4_rank.sql`
- Modify: `src/store/migrate.rs` (register M_V4, bump `CURRENT_VERSION` to "4", Rust simhash backfill)
- Test: `tests/store/migrate_v4.rs` (declare in `tests/store.rs`)

- [ ] **Step 1: Write the failing test**

`tests/store/migrate_v4.rs`:

```rust
use rusqlite::Connection;

/// Build a pre-v4 database by replaying the 0001..0003 SQL exactly as an
/// old binary would have, with one memory row + old-tokenizer FTS row.
fn build_v3_db(path: &std::path::Path) {
    let conn = Connection::open(path).expect("open raw");
    conn.execute_batch(comemory::store::migrate::M_BOOTSTRAP).expect("0001");
    conn.execute_batch(comemory::store::migrate::M_V2).expect("0002");
    conn.execute_batch(comemory::store::migrate::M_V3).expect("0003");
    conn.execute_batch(
        "INSERT INTO schema_meta(key, value) VALUES
            ('0002_v2_tables','1'), ('0003_stats_tables','1'), ('version','3');
         INSERT INTO memories(id, slug, kind, repo, author, quality, schema,
                              content_hash, body, created_at, updated_at, md_path)
         VALUES ('aabbccdd','vec-dim','bug','demo','f',3,1,'hash',
                 'the VecDimMismatch error fires on bad embedder dims',
                 '2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','memories/aabbccdd-vec-dim.md');
         INSERT INTO memory_fts(memory_id, body, tags)
         VALUES ('aabbccdd','the VecDimMismatch error fires on bad embedder dims','');",
    )
    .expect("seed v3 rows");
}

#[test]
fn open_migrates_v3_db_to_v4() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_v3_db(&db);

    let conn = comemory::store::connection::open(&db).expect("open migrates");

    // new columns exist with sane values
    let (count, last, sim): (i64, String, i64) = conn
        .query_row(
            "SELECT access_count, last_accessed, simhash FROM memories WHERE id='aabbccdd'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("columns present");
    assert_eq!(count, 0);
    assert_eq!(last, "2026-01-01T00:00:00Z"); // backfilled from created_at
    assert_ne!(sim, 0); // Rust backfill computed a real simhash

    // FTS rebuilt with identifier tokenizer: camelCase subtoken now matches
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'mismatch'",
            [],
            |r| r.get(0),
        )
        .expect("fts query");
    assert_eq!(hits, 1);

    // version bumped
    let v: String = conn
        .query_row("SELECT value FROM schema_meta WHERE key='version'", [], |r| r.get(0))
        .expect("version row");
    assert_eq!(v, "4");
}

#[test]
fn migration_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_v3_db(&db);
    drop(comemory::store::connection::open(&db).expect("first open"));
    let conn = comemory::store::connection::open(&db).expect("second open"); // must not error
    let hits: i64 = conn
        .query_row("SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'mismatch'", [], |r| r.get(0))
        .expect("fts still works");
    assert_eq!(hits, 1);
}
```

This requires `M_BOOTSTRAP`/`M_V2`/`M_V3` to become `pub` in `migrate.rs` (doc-commented).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --all-features -E 'binary(store)'`
Expected: compile FAILURE (consts private) then, after making them pub as part of this step's compile fix only, runtime FAILURE: no `access_count` column / version still "3".

- [ ] **Step 3: Implement migration**

`src/store/sql/0004_v4_rank.sql`:

```sql
-- v4: rank-blend core — access tracking, memory simhash, identifier-tokenized FTS.

ALTER TABLE memories ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN last_accessed TEXT;
ALTER TABLE memories ADD COLUMN simhash INTEGER NOT NULL DEFAULT 0;
ALTER TABLE code_symbols ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE code_symbols ADD COLUMN last_accessed TEXT;

UPDATE memories     SET last_accessed = created_at WHERE last_accessed IS NULL;
UPDATE code_symbols SET last_accessed = indexed_at WHERE last_accessed IS NULL;

-- Rebuild FTS tables with the identifier tokenizer. Content re-derives
-- from base tables, so DROP + CREATE + INSERT is safe and repeatable.
DROP TABLE memory_fts;
CREATE VIRTUAL TABLE memory_fts USING fts5(
    memory_id UNINDEXED,
    body,
    tags,
    tokenize = 'porter identifier'
);
INSERT INTO memory_fts(memory_id, body, tags)
SELECT m.id,
       m.body,
       COALESCE((SELECT group_concat(t.tag, ',')
                   FROM memory_tags t WHERE t.memory_id = m.id), '')
  FROM memories m
 WHERE m.deleted_at IS NULL;

DROP TABLE code_fts;
CREATE VIRTUAL TABLE code_fts USING fts5(
    symbol_id UNINDEXED,
    symbol,
    snippet,
    path_tokens,
    tokenize = 'identifier'
);
-- identifier tokenizer splits on '/', '.', '-' etc., so the raw path is
-- a valid path_tokens source.
INSERT INTO code_fts(symbol_id, symbol, snippet, path_tokens)
SELECT id, symbol, snippet, path FROM code_symbols;
```

`src/store/migrate.rs` changes:

```rust
/// Bootstrap migration: schema_meta table. Public so tests can replay
/// historical schema states.
pub const M_BOOTSTRAP: &str = include_str!("./sql/0001_schema_meta.sql");
/// v2 core tables (public for the same reason).
pub const M_V2: &str = include_str!("./sql/0002_v2_tables.sql");
/// v3 stats tables (public for the same reason).
pub const M_V3: &str = include_str!("./sql/0003_stats_tables.sql");
/// v4 rank-blend tables and FTS rebuild.
pub const M_V4: &str = include_str!("./sql/0004_v4_rank.sql");

pub const CURRENT_VERSION: &str = "4";
```

In `run()`, after the existing `apply` calls, apply 0004 and run the Rust backfill **only when it newly applied** — change `apply` to return `Result<bool>` (`true` = executed now):

```rust
let v4_applied = apply(conn, "0004_v4_rank", M_V4)?;
if v4_applied {
    backfill_memory_simhash(conn)?;
}
set_version(conn, CURRENT_VERSION)?;
```

```rust
/// Compute and store simhash for every memory that still has the
/// DEFAULT 0 placeholder (one-shot after the v4 migration; also heals
/// rows from interrupted runs).
fn backfill_memory_simhash(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id, body FROM memories WHERE simhash = 0")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    for (id, body) in rows {
        let hash = crate::simhash::simhash64(crate::simhash::tokens(&body));
        // SQLite INTEGER is i64; store the u64 bit pattern.
        conn.execute(
            "UPDATE memories SET simhash = ?1 WHERE id = ?2",
            rusqlite::params![hash as i64, id],
        )?;
    }
    Ok(())
}
```

Update the other `apply(...)` call sites for the new `Result<bool>` signature (ignore the bool with `let _ =` is forbidden style? No — just discard: `apply(conn, "0002_v2_tables", M_V2)?;` still compiles since `Result<bool>` with `?` yields `bool`; leave unused with `let _applied =` only where unneeded — simply `apply(...)?;` works, the bool drops).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --all-features -E 'binary(store)'`
Expected: PASS, including Task 2's `store_open_registers_tokenizer`.

- [ ] **Step 5: Full store + retrieval test sweep (regression: old tests must survive the new tokenizer)**

Run: `cargo nextest run --all-features`
Expected: PASS. If FTS-dependent tests assert old tokenization behavior, fix the *tests* only when the new behavior is the spec-approved one.

- [ ] **Step 6: Commit**

```bash
git add src/store/sql/0004_v4_rank.sql src/store/migrate.rs tests/store
git commit -m "feat(store): v4 migration — access tracking, memory simhash, identifier FTS rebuild"
```

---

### Task 4: Config — rank + prune knobs

**Files:**
- Modify: `src/config/file.rs` (new `RankConfig`, `PruneConfig` fields, env wiring)
- Test: extend `tests/config/` (mirror file for `file.rs` already exists — add cases there)

- [ ] **Step 1: Write the failing tests** (in the existing config test file for `file.rs`)

```rust
#[test]
fn rank_defaults() {
    let cfg = comemory::config::Config::defaults();
    assert_eq!(cfg.rank.decay, 0.5);
    assert_eq!(cfg.rank.prior_clamp, (0.5, 2.0));
    assert_eq!(cfg.rank.mmr_lambda, 0.7);
    assert_eq!(cfg.prune.min_activation, -2.0);
    assert_eq!(cfg.prune.min_feedback, 0.25);
}

#[test]
fn rank_env_overrides() {
    // follow the existing env-test pattern in this file (serial / scoped env)
    std::env::set_var("COMEMORY_RANK_DECAY", "0.7");
    std::env::set_var("COMEMORY_RANK_PRIOR_CLAMP", "0.6,1.8");
    std::env::set_var("COMEMORY_RANK_MMR_LAMBDA", "0.5");
    std::env::set_var("COMEMORY_PRUNE_MIN_ACTIVATION", "-1.5");
    std::env::set_var("COMEMORY_PRUNE_MIN_FEEDBACK", "0.3");
    let cfg = comemory::config::Config::defaults().with_env().expect("env ok");
    assert_eq!(cfg.rank.decay, 0.7);
    assert_eq!(cfg.rank.prior_clamp, (0.6, 1.8));
    assert_eq!(cfg.rank.mmr_lambda, 0.5);
    assert_eq!(cfg.prune.min_activation, -1.5);
    assert_eq!(cfg.prune.min_feedback, 0.3);
    std::env::remove_var("COMEMORY_RANK_DECAY");
    std::env::remove_var("COMEMORY_RANK_PRIOR_CLAMP");
    std::env::remove_var("COMEMORY_RANK_MMR_LAMBDA");
    std::env::remove_var("COMEMORY_PRUNE_MIN_ACTIVATION");
    std::env::remove_var("COMEMORY_PRUNE_MIN_FEEDBACK");
}

#[test]
fn bad_clamp_is_an_error() {
    std::env::set_var("COMEMORY_RANK_PRIOR_CLAMP", "2.0,0.5"); // lo > hi
    assert!(comemory::config::Config::defaults().with_env().is_err());
    std::env::remove_var("COMEMORY_RANK_PRIOR_CLAMP");
}
```

(If the existing env tests use a mutex/serial guard for env mutation, reuse it — copy the established pattern in that file.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --all-features -E 'binary(config)'`
Expected: compile FAILURE — no `rank` field.

- [ ] **Step 3: Implement config**

In `src/config/file.rs`, following the existing sub-config struct style exactly (serde derives, `Default` via `defaults()`):

```rust
/// Ranking knobs for the rerank/diversify pipeline (M1).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RankConfig {
    /// ACT-R decay exponent `d` in `ln(n) − d·ln(days + 1)`.
    pub decay: f64,
    /// Bounds applied to every rerank prior multiplier.
    pub prior_clamp: (f64, f64),
    /// MMR relevance-vs-diversity trade-off (1.0 = pure relevance).
    pub mmr_lambda: f64,
}
```

Defaults in `Config::defaults()`: `rank: RankConfig { decay: 0.5, prior_clamp: (0.5, 2.0), mmr_lambda: 0.7 }`.

`PruneConfig` gains:

```rust
    /// Activation floor below which a memory is prune-eligible.
    pub min_activation: f64,
    /// Beta-feedback ceiling at/below which a memory is prune-eligible.
    pub min_feedback: f64,
```

defaults `-2.0` / `0.25`.

`with_env()` additions, following the existing parse-or-`Error::Config` pattern at `file.rs:201-240`:

- `COMEMORY_RANK_DECAY` → f64, error if non-finite or < 0.
- `COMEMORY_RANK_PRIOR_CLAMP` → split on ',', parse two f64, error unless both finite and `lo <= hi` and `lo > 0`.
- `COMEMORY_RANK_MMR_LAMBDA` → f64 in `[0,1]`, else error.
- `COMEMORY_PRUNE_MIN_ACTIVATION` → f64, finite.
- `COMEMORY_PRUNE_MIN_FEEDBACK` → f64 in `[0,1]`.
- Also wire the two existing file-only knobs: `COMEMORY_PRUNE_BELOW_QUALITY` → u8 in `1..=5`, `COMEMORY_PRUNE_UNUSED_SINCE_DAYS` → u32.

If the optional `config.toml` partial-config struct exists for other sections, add matching optional fields there too (same pattern as `retrieval`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --all-features -E 'binary(config)'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/config tests/config
git commit -m "feat(config): rank + prune scoring knobs with env wiring"
```

---

### Task 5: Score primitives (activation, Beta feedback, boosts)

**Files:**
- Create: `src/retrieval/score.rs`
- Modify: `src/retrieval/mod.rs` (add `pub mod score;`)
- Test: `tests/retrieval/score.rs` (declare in `tests/retrieval.rs`)

- [ ] **Step 1: Write the failing tests**

`tests/retrieval/score.rs`:

```rust
use comemory::retrieval::score::*;
use proptest::prelude::*;

const CLAMP: (f64, f64) = (0.5, 2.0);

#[test]
fn fresh_memory_is_neutral() {
    // n=1 (created counts as first access), same-day: activation 0 → boost 1.0
    let a = activation(0, 0.0, 0.5); // access_count 0 is floored to 1
    assert_eq!(a, 0.0);
    assert_eq!(activation_boost(a, CLAMP), 1.0);
}

#[test]
fn zero_feedback_is_neutral() {
    let b = beta_feedback(0, 0);
    assert_eq!(b, 0.25);
    assert_eq!(feedback_boost(b, CLAMP), 1.0);
}

#[test]
fn quality_three_is_neutral() {
    assert_eq!(quality_boost(3, CLAMP), 1.0);
}

#[test]
fn old_unaccessed_memory_sinks_below_threshold() {
    // single access 90 days ago ≈ −2.26 < default prune floor −2.0
    let a = activation(1, 90.0, 0.5);
    assert!(a < -2.0, "got {a}");
}

proptest! {
    #[test]
    fn activation_monotone_in_count(n in 1u64..10_000, days in 0.0f64..3650.0) {
        prop_assert!(activation(n + 1, days, 0.5) >= activation(n, days, 0.5));
    }

    #[test]
    fn activation_decays_with_time(n in 1u64..10_000, days in 0.0f64..3650.0) {
        prop_assert!(activation(n, days + 1.0, 0.5) <= activation(n, days, 0.5));
    }

    #[test]
    fn irrelevant_votes_never_raise_feedback(u in 0u64..1000, i in 0u64..1000) {
        prop_assert!(beta_feedback(u, i + 1) <= beta_feedback(u, i));
    }

    #[test]
    fn boosts_always_within_clamp(a in -100.0f64..100.0, b in 0.0f64..1.0, q in 1u8..=5) {
        for v in [activation_boost(a, CLAMP), feedback_boost(b, CLAMP), quality_boost(q, CLAMP)] {
            prop_assert!(v.is_finite());
            prop_assert!((CLAMP.0..=CLAMP.1).contains(&v));
        }
    }

    #[test]
    fn no_nan_ever(n in 0u64..u64::MAX, days in -10.0f64..1.0e9, d in 0.0f64..10.0) {
        prop_assert!(activation(n, days, d).is_finite());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --all-features -E 'binary(retrieval)'`
Expected: compile FAILURE — module missing.

- [ ] **Step 3: Implement**

`src/retrieval/score.rs` (complete file):

```rust
//! Deterministic scoring primitives: ACT-R activation (Petrov
//! approximation), Beta-smoothed feedback, and the bounded multiplier
//! mappings used by the rerank stage. Pure functions — time and counts
//! come in as arguments so tests stay clock-free.

/// ACT-R base-level activation, Petrov approximation:
/// `ln(max(n,1)) − d·ln(max(days,0) + 1)`. Time is measured in days; the
/// `+ 1` keeps the value finite for same-day access.
pub fn activation(access_count: u64, days_since_access: f64, decay: f64) -> f64 {
    let n = access_count.max(1) as f64;
    let days = if days_since_access.is_finite() { days_since_access.max(0.0) } else { 0.0 };
    n.ln() - decay * (days + 1.0).ln()
}

/// Posterior mean of Beta(1, 3) prior over used/irrelevant feedback:
/// `(used + 1) / (used + irrelevant + 4)`. Zero feedback → 0.25.
pub fn beta_feedback(used: u64, irrelevant: u64) -> f64 {
    (used as f64 + 1.0) / ((used + irrelevant) as f64 + 4.0)
}

/// Map activation to a bounded multiplier; activation 0 → 1.0.
pub fn activation_boost(activation: f64, clamp: (f64, f64)) -> f64 {
    bounded((0.2 * activation).exp(), clamp)
}

/// Map Beta feedback to a bounded multiplier; the 0.25 neutral point → 1.0.
pub fn feedback_boost(beta: f64, clamp: (f64, f64)) -> f64 {
    bounded(beta / 0.25, clamp)
}

/// Map quality 1..=5 to a bounded multiplier; quality 3 → 1.0.
pub fn quality_boost(quality: u8, clamp: (f64, f64)) -> f64 {
    bounded(1.0 + 0.075 * (f64::from(quality) - 3.0), clamp)
}

/// Fixed multiplier applied to results superseded by a live memory.
pub const SUPERSEDE_PENALTY: f64 = 0.2;

fn bounded(v: f64, (lo, hi): (f64, f64)) -> f64 {
    if !v.is_finite() {
        return 1.0;
    }
    v.max(lo).min(hi)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --all-features -E 'binary(retrieval)'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/retrieval/score.rs src/retrieval/mod.rs tests/retrieval
git commit -m "feat(retrieval): ACT-R activation, Beta feedback, bounded boost primitives"
```

---

### Task 6: Rerank stage

**Files:**
- Create: `src/retrieval/rerank.rs`
- Modify: `src/retrieval/mod.rs` (add `pub mod rerank;`)
- Test: `tests/retrieval/rerank.rs` (declare in `tests/retrieval.rs`)

- [ ] **Step 1: Write the failing test**

`tests/retrieval/rerank.rs`:

```rust
use comemory::retrieval::rerank::{rerank, Reranked};
use comemory::retrieval::router::{RoutedHit, Source};

fn open_seeded() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("comemory.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, access_count, last_accessed, simhash)
         VALUES
         ('aaaa0001','one','note','demo','f',3,1,'h1','first body',
          '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/1.md',0,'2026-06-09T00:00:00Z',1),
         ('aaaa0002','two','note','demo','f',5,1,'h2','second body',
          '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/2.md',0,'2026-06-09T00:00:00Z',2),
         ('aaaa0003','old','note','demo','f',3,1,'h3','third body',
          '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/3.md',0,'2026-06-09T00:00:00Z',3);
         INSERT INTO feedback(memory_id, used_count, irrelevant_count)
         VALUES ('aaaa0003', 0, 20);
         -- aaaa0002 supersedes aaaa0001
         INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0002','memory','aaaa0001','supersedes','2026-06-09T00:00:00Z');",
    )
    .expect("seed");
    (dir, conn)
}

fn hit(id: &str, score: f32) -> RoutedHit {
    RoutedHit { memory_id: id.into(), score, source: Source::Lexical }
}

#[test]
fn priors_reorder_equal_relevance() {
    let (_d, conn) = open_seeded();
    let cfg = comemory::config::Config::defaults();
    let hits = vec![hit("aaaa0001", 1.0), hit("aaaa0002", 1.0), hit("aaaa0003", 1.0)];
    let out: Vec<Reranked> = rerank(&conn, &cfg, &hits).expect("rerank");
    // quality-5 un-superseded memory first; heavily-downvoted last? No —
    // superseded ×0.2 beats feedback floor 0.5: aaaa0001 last.
    assert_eq!(out[0].memory_id, "aaaa0002");
    assert_eq!(out.last().expect("nonempty").memory_id, "aaaa0001");
}

#[test]
fn superseded_hit_is_annotated_and_penalized() {
    let (_d, conn) = open_seeded();
    let cfg = comemory::config::Config::defaults();
    let out = rerank(&conn, &cfg, &[hit("aaaa0001", 1.0)]).expect("rerank");
    assert_eq!(out[0].superseded_by.as_deref(), Some("aaaa0002"));
    assert!((out[0].parts.supersede - 0.2).abs() < 1e-9);
    assert!(out[0].parts.final_score < 0.3);
}

#[test]
fn score_parts_multiply_to_final() {
    let (_d, conn) = open_seeded();
    let cfg = comemory::config::Config::defaults();
    let out = rerank(&conn, &cfg, &[hit("aaaa0002", 0.8)]).expect("rerank");
    let p = &out[0].parts;
    let expect = f64::from(p.rrf) * p.activation * p.feedback * p.quality * p.supersede;
    assert!((p.final_score - expect).abs() < 1e-9);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --all-features -E 'binary(retrieval)'`
Expected: compile FAILURE.

- [ ] **Step 3: Implement**

`src/retrieval/rerank.rs` (complete file):

```rust
//! Second retrieval stage: multiply the fused relevance score by bounded
//! deterministic priors (activation, feedback, quality, supersede) and
//! expose every factor as `score_parts` for explainability.

use rusqlite::{Connection, OptionalExtension};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::router::{RoutedHit, Source};
use crate::retrieval::score;

/// Multiplicative factors behind a final score. Serialized verbatim into
/// `--json` output — a stable contract, not debug info.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScoreParts {
    /// Fused relevance from the candidate stage (RRF / lexical / vector).
    pub rrf: f32,
    /// ACT-R activation boost.
    pub activation: f64,
    /// Beta-smoothed feedback boost.
    pub feedback: f64,
    /// Frontmatter quality boost.
    pub quality: f64,
    /// 0.2 when superseded by a live memory, else 1.0.
    pub supersede: f64,
    /// Product of all factors.
    pub final_score: f64,
}

/// A reranked hit, ready for the diversity stage.
#[derive(Debug, Clone)]
pub struct Reranked {
    pub memory_id: String,
    pub source: Source,
    pub parts: ScoreParts,
    /// Live memory that supersedes this one, if any.
    pub superseded_by: Option<String>,
    /// Body text, carried for MMR/SimHash in the diversify stage.
    pub body: String,
    /// SimHash of the body, carried for near-dup collapse.
    pub simhash: u64,
}

/// Rerank candidates by multiplying relevance with bounded priors.
/// Hits whose memory row vanished (raced delete) are dropped.
pub fn rerank(conn: &Connection, cfg: &Config, hits: &[RoutedHit]) -> Result<Vec<Reranked>> {
    let now = OffsetDateTime::now_utc();
    let clamp = cfg.rank.prior_clamp;
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        let Some(row) = memory_signals(conn, &hit.memory_id)? else {
            continue;
        };
        let days = days_between(&row.last_accessed, now);
        let act = score::activation(row.access_count, days, cfg.rank.decay);
        let beta = score::beta_feedback(row.used, row.irrelevant);
        let superseded_by = live_superseder(conn, &hit.memory_id)?;
        let supersede = if superseded_by.is_some() { score::SUPERSEDE_PENALTY } else { 1.0 };
        let parts = ScoreParts {
            rrf: hit.score,
            activation: score::activation_boost(act, clamp),
            feedback: score::feedback_boost(beta, clamp),
            quality: score::quality_boost(row.quality, clamp),
            supersede,
            final_score: 0.0,
        };
        let final_score =
            f64::from(parts.rrf) * parts.activation * parts.feedback * parts.quality * parts.supersede;
        out.push(Reranked {
            memory_id: hit.memory_id.clone(),
            source: hit.source,
            parts: ScoreParts { final_score, ..parts },
            superseded_by,
            body: row.body,
            simhash: row.simhash,
        });
    }
    out.sort_by(|a, b| {
        b.parts
            .final_score
            .partial_cmp(&a.parts.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

struct Signals {
    quality: u8,
    access_count: u64,
    last_accessed: String,
    body: String,
    simhash: u64,
    used: u64,
    irrelevant: u64,
}

fn memory_signals(conn: &Connection, id: &str) -> Result<Option<Signals>> {
    conn.query_row(
        "SELECT m.quality, m.access_count, COALESCE(m.last_accessed, m.created_at),
                m.body, m.simhash,
                COALESCE(f.used_count, 0), COALESCE(f.irrelevant_count, 0)
           FROM memories m
           LEFT JOIN feedback f ON f.memory_id = m.id
          WHERE m.id = ?1 AND m.deleted_at IS NULL",
        [id],
        |r| {
            Ok(Signals {
                quality: r.get(0)?,
                access_count: r.get::<_, i64>(1)?.max(0) as u64,
                last_accessed: r.get(2)?,
                body: r.get(3)?,
                simhash: r.get::<_, i64>(4)? as u64,
                used: r.get::<_, i64>(5)?.max(0) as u64,
                irrelevant: r.get::<_, i64>(6)?.max(0) as u64,
            })
        },
    )
    .optional()
    .map_err(Error::from)
}

fn live_superseder(conn: &Connection, id: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT e.src_id FROM edges e
           JOIN memories m ON m.id = e.src_id AND m.deleted_at IS NULL
          WHERE e.rel = 'supersedes' AND e.dst_kind = 'memory' AND e.dst_id = ?1
          LIMIT 1",
        [id],
        |r| r.get(0),
    )
    .optional()
    .map_err(Error::from)
}

fn days_between(rfc3339: &str, now: OffsetDateTime) -> f64 {
    match OffsetDateTime::parse(rfc3339, &Rfc3339) {
        Ok(then) => ((now - then).whole_seconds() as f64 / 86_400.0).max(0.0),
        Err(_) => 0.0, // unparsable timestamp → treat as fresh, never punish
    }
}
```

Note: `out.sort_by` uses `partial_cmp(...).unwrap_or(...)` — **not** `.unwrap()`, passes the no-bypass gate. `RoutedHit`/`Source` must be `pub` with `Clone`/`Copy` as needed (check `router.rs`; add `#[derive(Clone, Copy)]` to `Source` if missing).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run --all-features -E 'binary(retrieval)'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/retrieval tests/retrieval
git commit -m "feat(retrieval): rerank stage with bounded multiplicative priors + score_parts"
```

---

### Task 7: Diversify stage (SimHash collapse + MMR)

**Files:**
- Create: `src/retrieval/diversify.rs`
- Modify: `src/retrieval/mod.rs` (add `pub mod diversify;`)
- Test: `tests/retrieval/diversify.rs` (declare in `tests/retrieval.rs`)

- [ ] **Step 1: Write the failing test**

`tests/retrieval/diversify.rs`:

```rust
use comemory::retrieval::diversify::diversify;
use comemory::retrieval::rerank::{Reranked, ScoreParts};
use comemory::retrieval::router::Source;

fn item(id: &str, score: f64, body: &str) -> Reranked {
    Reranked {
        memory_id: id.into(),
        source: Source::Lexical,
        parts: ScoreParts {
            rrf: score as f32,
            activation: 1.0,
            feedback: 1.0,
            quality: 1.0,
            supersede: 1.0,
            final_score: score,
        },
        superseded_by: None,
        body: body.into(),
        simhash: comemory::simhash::simhash64(comemory::simhash::tokens(body)),
    }
}

#[test]
fn near_duplicates_collapse_to_best_scored() {
    let a = item("aaaa0001", 0.9, "postgres connection pool exhausted under load");
    let b = item("aaaa0002", 0.5, "postgres connection pool exhausted under heavy load");
    let c = item("aaaa0003", 0.7, "rustfmt disagrees with clippy about line width");
    let out = diversify(vec![a, b, c], 0.7, 10);
    let ids: Vec<&str> = out.iter().map(|r| r.memory_id.as_str()).collect();
    assert!(ids.contains(&"aaaa0001"), "best dup kept");
    assert!(!ids.contains(&"aaaa0002"), "worse dup collapsed");
    assert!(ids.contains(&"aaaa0003"));
}

#[test]
fn mmr_prefers_diverse_over_marginally_better() {
    // two near-identical topics + one distinct; k=2 must pick one of each
    let a = item("aaaa0001", 0.9, "sqlite fts5 tokenizer registration order");
    let b = item("aaaa0002", 0.89, "sqlite fts5 tokenizer registration sequence and order details");
    let c = item("aaaa0003", 0.6, "git hooks install path on windows runners");
    // make a/b NOT simhash-near (different enough bodies) but token-similar:
    // if simhash collapses them anyway, weaken the assertion to len==2 distinct topics.
    let out = diversify(vec![a, b, c], 0.7, 2);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].memory_id, "aaaa0001");
    assert_eq!(out[1].memory_id, "aaaa0003");
}

#[test]
fn truncates_to_top_k() {
    let items: Vec<_> = (0..30)
        .map(|i| item(&format!("aaaa{i:04}"), 1.0 - i as f64 * 0.01, &format!("unique body {i} about topic {i}")))
        .collect();
    assert_eq!(diversify(items, 0.7, 12).len(), 12);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --all-features -E 'binary(retrieval)'`
Expected: compile FAILURE.

- [ ] **Step 3: Implement**

`src/retrieval/diversify.rs` (complete file):

```rust
//! Third retrieval stage: collapse SimHash near-duplicates, then apply
//! MMR (maximal marginal relevance) with token-set Jaccard similarity,
//! and cut to top-k. Embedding-free by design.

use std::collections::HashSet;

use crate::retrieval::rerank::Reranked;
use crate::simhash::hamming64;

/// Hamming radius treated as "same memory, different wording".
const NEAR_DUP_HAMMING: u32 = 3;

/// Collapse near-duplicates, then greedily select up to `top_k` items
/// maximizing `lambda·score − (1−lambda)·max_jaccard_to_selected`.
/// Input must already be sorted by final score descending (rerank output).
pub fn diversify(items: Vec<Reranked>, lambda: f64, top_k: usize) -> Vec<Reranked> {
    let deduped = collapse_near_dups(items);
    mmr(deduped, lambda, top_k)
}

fn collapse_near_dups(items: Vec<Reranked>) -> Vec<Reranked> {
    let mut kept: Vec<Reranked> = Vec::with_capacity(items.len());
    for item in items {
        // items arrive best-first, so the first of a dup group wins
        let dup = kept.iter().any(|k| hamming64(k.simhash, item.simhash) <= NEAR_DUP_HAMMING);
        if !dup {
            kept.push(item);
        }
    }
    kept
}

fn token_set(body: &str) -> HashSet<String> {
    crate::simhash::tokens(body).into_iter().collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    inter / union
}

fn mmr(items: Vec<Reranked>, lambda: f64, top_k: usize) -> Vec<Reranked> {
    let sets: Vec<HashSet<String>> = items.iter().map(|i| token_set(&i.body)).collect();
    let mut remaining: Vec<usize> = (0..items.len()).collect();
    let mut picked_idx: Vec<usize> = Vec::with_capacity(top_k.min(items.len()));
    while picked_idx.len() < top_k && !remaining.is_empty() {
        let (pos, &best) = remaining
            .iter()
            .enumerate()
            .max_by(|(_, &a), (_, &b)| {
                mmr_score(&items, &sets, &picked_idx, a, lambda)
                    .partial_cmp(&mmr_score(&items, &sets, &picked_idx, b, lambda))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or((0, &0));
        picked_idx.push(best);
        remaining.remove(pos);
    }
    // map indices back to owned items, preserving pick order
    let mut slots: Vec<Option<Reranked>> = items.into_iter().map(Some).collect();
    picked_idx.into_iter().filter_map(|i| slots[i].take()).collect()
}

fn mmr_score(
    items: &[Reranked],
    sets: &[HashSet<String>],
    picked: &[usize],
    candidate: usize,
    lambda: f64,
) -> f64 {
    let max_sim = picked
        .iter()
        .map(|&p| jaccard(&sets[candidate], &sets[p]))
        .fold(0.0f64, f64::max);
    lambda * items[candidate].parts.final_score - (1.0 - lambda) * max_sim
}
```

(`unwrap_or((0, &0))` on the `max_by` of a non-empty slice is unreachable-defensive, not `.unwrap()` — gate-safe. `src/simhash.rs` fns are already `pub`.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run --all-features -E 'binary(retrieval)'`
Expected: PASS. If `mmr_prefers_diverse...` fails because SimHash collapse already removed item b, adjust b's body to differ more (the test comment anticipates this) while keeping high token overlap.

- [ ] **Step 5: Commit**

```bash
git add src/retrieval tests/retrieval
git commit -m "feat(retrieval): diversify stage — simhash collapse + jaccard MMR"
```

---

### Task 8: Candidate-stage upgrades — BM25 weights, prefix match, relaxed tier

**Files:**
- Modify: `src/store/fts.rs` (match-query builder, weighted bm25, relaxed variant)
- Modify: `src/retrieval/router.rs` (candidate pool size, relaxed fallback)
- Test: extend `tests/store/fts.rs` (or the existing mirror file for `fts.rs`) and `tests/retrieval/router.rs` (existing mirrors)

- [ ] **Step 1: Write the failing tests**

In the fts mirror test file, add:

```rust
#[test]
fn build_match_query_quotes_and_prefixes_last_term() {
    assert_eq!(
        comemory::store::fts::build_match_query("vec dim mism"),
        r#""vec" "dim" "mism"*"#
    );
    // embedded quotes are stripped, never injected into FTS syntax
    assert_eq!(
        comemory::store::fts::build_match_query(r#"a"b"#),
        r#""ab"*"#
    );
    assert_eq!(comemory::store::fts::build_match_query(""), "");
}

#[test]
fn build_or_query_joins_terms() {
    assert_eq!(
        comemory::store::fts::build_or_query("auth login race"),
        r#""auth" OR "login" OR "race""#
    );
}

#[test]
fn tag_match_outranks_body_match() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash)
         VALUES ('aaaa0001','a','note','d','f',3,1,'h1','postgres mentioned once in body',
                 '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/1.md',1),
                ('aaaa0002','b','note','d','f',3,1,'h2','completely unrelated body text',
                 '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/2.md',2);
         INSERT INTO memory_fts(memory_id, body, tags)
         VALUES ('aaaa0001','postgres mentioned once in body',''),
                ('aaaa0002','completely unrelated body text','postgres');",
    ).expect("seed");
    let hits = comemory::store::fts::search_memory(&conn, "postgres", 10, None).expect("search");
    assert_eq!(hits[0].memory_id, "aaaa0002", "tag hit must outrank body hit");
}
```

In the router mirror test file, add:

```rust
#[test]
fn relaxed_fallback_fires_when_strict_and_finds_partial_terms() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash)
         VALUES ('aaaa0001','a','note','d','f',3,1,'h1','the oauth refresh race condition',
                 '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/1.md',1);
         INSERT INTO memory_fts(memory_id, body, tags)
         VALUES ('aaaa0001','the oauth refresh race condition','');",
    ).expect("seed");
    let cfg = comemory::config::Config::defaults();
    // strict AND of all three terms fails ('login' absent) → OR tier finds it
    let hits = comemory::retrieval::router::route(&cfg, &conn, "oauth login race", None, None)
        .expect("route");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "aaaa0001");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --all-features -E 'binary(store) or binary(retrieval)'`
Expected: compile FAILURE (`build_match_query` missing) / assertion FAILURE (tag weighting, fallback).

- [ ] **Step 3: Implement**

`src/store/fts.rs` additions/changes:

```rust
/// Build a strict FTS5 MATCH query: every whitespace term double-quoted
/// (quotes stripped from input — terms are data, never syntax), last term
/// prefix-matched.
pub fn build_match_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| !t.is_empty())
        .collect();
    let n = terms.len();
    terms
        .iter()
        .enumerate()
        .map(|(i, t)| if i + 1 == n { format!("\"{t}\"*") } else { format!("\"{t}\"") })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build a relaxed OR query over the same sanitized terms (no prefixing).
pub fn build_or_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}
```

In `search_memory`, change the MATCH argument from the raw query to `build_match_query(query)` and the ORDER BY to weighted bm25 — columns are `(memory_id UNINDEXED, body, tags)`, so:

```sql
ORDER BY bm25(memory_fts, 0.0, 1.0, 3.0)
```

Add a relaxed variant (same SQL, query built with `build_or_query`):

```rust
/// Relaxed lexical search: OR-joined terms, used as the corrective
/// fallback tier when the strict query returns nothing.
pub fn search_memory_relaxed(
    conn: &Connection,
    query: &str,
    k: usize,
    repo: Option<&str>,
) -> Result<Vec<MemoryFtsHit>> { /* same body as search_memory but with build_or_query */ }
```

Extract the shared SQL execution into a private helper (`fn run_memory_match(conn, match_expr, k, repo)`) so the two pub fns are thin wrappers — dup-check gate (`scripts/dup-check.sh`) requires it.

Apply the same `build_match_query` treatment to `search_code` (weights: `bm25(code_fts, 0.0, 2.0, 1.0, 1.5)` — symbol weighted highest, then path_tokens, then snippet).

`src/retrieval/router.rs` changes:

```rust
/// Candidate pool fed to the rerank stage; the pipeline cuts to top_k
/// after diversification.
pub const CANDIDATE_POOL: usize = 50;
```

- Wherever `route` currently uses `cfg.retrieval.top_k` for the fetch size, use `CANDIDATE_POOL.max(cfg.retrieval.top_k)` instead (final truncation moves to the diversify stage).
- At the end of the lexical and hybrid paths: if the result is empty and the query has ≥2 terms, retry the lexical branch with `fts::search_memory_relaxed` and return those hits (`Source::Lexical`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --all-features -E 'binary(store) or binary(retrieval)'`
Expected: PASS.

- [ ] **Step 5: Full suite + dup check + commit**

Run: `cargo nextest run --all-features && bash scripts/dup-check.sh`
Expected: PASS (existing router/search tests may need top-k expectation updates — candidate pool is larger now; fix tests to assert against pipeline output, not route output, where they asserted exact lengths).

```bash
git add src/store/fts.rs src/retrieval/router.rs tests/store tests/retrieval
git commit -m "feat(retrieval): weighted bm25, prefix matching, relaxed OR fallback tier"
```

---

### Task 9: Pipeline wiring — search command, access updates, score_parts output

**Files:**
- Create: `src/retrieval/pipeline.rs`
- Modify: `src/retrieval/mod.rs` (add `pub mod pipeline;`)
- Modify: `src/cli/search.rs` (call pipeline instead of bare route)
- Modify: `src/output/search.rs` (emit score_parts + superseded_by)
- Test: `tests/retrieval/pipeline.rs`, extend `tests/output/` mirror for search, snapshot test

- [ ] **Step 1: Write the failing tests**

`tests/retrieval/pipeline.rs`:

```rust
use comemory::retrieval::pipeline::search;

fn seeded() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash)
         VALUES ('aaaa0001','a','note','d','f',3,1,'h1','sqlite busy timeout fix for pool',
                 '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/1.md',1);
         INSERT INTO memory_fts(memory_id, body, tags)
         VALUES ('aaaa0001','sqlite busy timeout fix for pool','');",
    ).expect("seed");
    (dir, conn)
}

#[test]
fn search_returns_reranked_diversified_hits() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let out = search(&cfg, &conn, "sqlite busy", None, None).expect("search");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].memory_id, "aaaa0001");
    assert!(out[0].parts.final_score > 0.0);
}

#[test]
fn retrieval_hit_bumps_access_tracking() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    search(&cfg, &conn, "sqlite busy", None, None).expect("search");
    let (count, last): (i64, String) = conn
        .query_row(
            "SELECT access_count, last_accessed FROM memories WHERE id='aaaa0001'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(count, 1);
    assert!(last > "2026-06-09T00:00:00Z".to_string(), "last_accessed updated, got {last}");
}
```

Output snapshot (in the existing output test module for search):

```rust
#[test]
fn search_json_envelope_contract() {
    use comemory::retrieval::rerank::{Reranked, ScoreParts};
    use comemory::retrieval::router::Source;
    let hits = vec![Reranked {
        memory_id: "aaaa0001".into(),
        source: Source::Hybrid,
        parts: ScoreParts {
            rrf: 0.016,
            activation: 1.0,
            feedback: 1.0,
            quality: 1.0,
            supersede: 1.0,
            final_score: 0.016,
        },
        superseded_by: None,
        body: String::new(),
        simhash: 0,
    }];
    insta::assert_json_snapshot!(comemory::output::search::envelope(&hits));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --all-features -E 'binary(retrieval) or binary(output)'`
Expected: compile FAILURE.

- [ ] **Step 3: Implement**

`src/retrieval/pipeline.rs` (complete file):

```rust
//! End-to-end memory search: route (candidates) → rerank (priors) →
//! diversify (dedup + MMR) → top-k, plus best-effort access tracking.

use rusqlite::Connection;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::rerank::Reranked;
use crate::retrieval::{diversify, rerank, router};

/// Run the full retrieval pipeline for a memory query.
pub fn search(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: Option<&[f32]>,
    repo: Option<&str>,
) -> Result<Vec<Reranked>> {
    let candidates = router::route(cfg, conn, query, vec, repo)?;
    let reranked = rerank::rerank(conn, cfg, &candidates)?;
    let final_hits = diversify::diversify(reranked, cfg.rank.mmr_lambda, cfg.retrieval.top_k);
    record_access(conn, &final_hits);
    Ok(final_hits)
}

/// Bump access tracking for returned hits. Best-effort: a failure must
/// never break the read path.
fn record_access(conn: &Connection, hits: &[Reranked]) {
    let now = match OffsetDateTime::now_utc().format(&Rfc3339) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "access tracking skipped: timestamp format failed");
            return;
        }
    };
    for hit in hits {
        if let Err(e) = conn.execute(
            "UPDATE memories SET access_count = access_count + 1, last_accessed = ?1 WHERE id = ?2",
            rusqlite::params![now, hit.memory_id],
        ) {
            tracing::warn!(error = %e, id = %hit.memory_id, "access tracking update failed");
        }
    }
}
```

`src/cli/search.rs`: replace the `router::route(...)` call with `retrieval::pipeline::search(&cfg, &conn, &a.query, vec.as_deref(), a.repo.as_deref())` and pass the result to the updated emitter.

`src/output/search.rs`: rework around `Reranked`:

```rust
/// One search hit as emitted to the user.
#[derive(serde::Serialize)]
pub struct Row<'a> {
    pub memory_id: &'a str,
    pub score: f64,
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<&'a str>,
    pub score_parts: &'a crate::retrieval::rerank::ScoreParts,
}

/// JSON envelope for `comemory search --json`.
#[derive(serde::Serialize)]
pub struct Envelope<'a> {
    pub hits: Vec<Row<'a>>,
}

/// Build the serializable envelope (public so snapshot tests can pin the contract).
pub fn envelope(hits: &[Reranked]) -> Envelope<'_> { /* map fields, source label as today */ }

/// Emit hits as JSON or TTY.
pub fn emit(hits: &[Reranked], json_flag: bool) -> Result<()> {
    if json_flag {
        return crate::output::json::write(&envelope(hits));
    }
    // TTY: existing score/source/id line; append " (superseded by <id>)" when set
    ...
}
```

Keep the TTY line format identical to today's (`{score}  {source}  {memory_id}`) with the optional supersede suffix, so human output stays familiar.

Check `comemory context` (`src/cli/context.rs`): if it calls `router::route` for memory headlines, switch it to `pipeline::search` too — one retrieval path everywhere (`bundle::assemble` keeps taking memory ids, unchanged).

- [ ] **Step 4: Run tests to verify they pass; review + accept snapshots**

Run: `cargo nextest run --all-features`
Expected: PASS after `cargo insta review` (or `cargo insta accept` once diff inspected). Existing search-output tests will need updating to the new envelope — that's the approved contract change.

- [ ] **Step 5: Commit**

```bash
git add src/retrieval src/cli/search.rs src/cli/context.rs src/output/search.rs tests
git commit -m "feat(retrieval): wire route→rerank→diversify pipeline with access tracking + score_parts output"
```

---

### Task 10: Save-time duplicate warning + simhash persistence

**Files:**
- Modify: `src/store/memory_row.rs` (persist simhash on insert)
- Modify: `src/cli/save.rs` (dup check before save, `duplicate_of` in output)
- Modify: `src/output/` save emitter (warning line)
- Test: extend the mirrors for `memory_row.rs` and `save.rs`; integration case in `tests/cli.rs`

- [ ] **Step 1: Write the failing tests**

Integration (in `tests/cli.rs`, following its existing `bin(&home)` helper pattern):

```rust
#[test]
fn near_duplicate_save_warns_and_hints() {
    let home = sandbox(); // existing helper naming may differ — reuse it
    let first = bin(&home)
        .args(["--json", "save", "postgres pool exhausted under load spikes", "--kind", "bug"])
        .assert()
        .success();
    let first_id = extract_saved_id(&first); // existing helper

    let second = bin(&home)
        .args(["--json", "save", "postgres pool exhausted under heavy load spikes", "--kind", "bug"])
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_slice(&second.get_output().stdout).expect("json");
    assert_eq!(v["duplicate_of"].as_str(), Some(first_id.as_str()));

    // distinct content has no hint
    let third = bin(&home)
        .args(["--json", "save", "ast-grep pattern for tokio spawn blocks", "--kind", "pattern"])
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_slice(&third.get_output().stdout).expect("json");
    assert!(v["duplicate_of"].is_null());
}
```

Unit-level (memory_row mirror): after `insert`, `SELECT simhash FROM memories WHERE id=?` is nonzero and equals `simhash64(tokens(body))`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --all-features -E 'binary(cli)'`
Expected: FAILURE — `duplicate_of` key absent.

- [ ] **Step 3: Implement**

`src/store/memory_row.rs::insert`: compute and persist simhash (add the column to the INSERT/upsert statement):

```rust
let simhash = crate::simhash::simhash64(crate::simhash::tokens(body)) as i64;
```

`src/cli/save.rs`:

```rust
/// Find a live memory whose body simhash is within near-dup range.
fn near_duplicate(conn: &rusqlite::Connection, body: &str) -> Option<String> {
    let hash = crate::simhash::simhash64(crate::simhash::tokens(body));
    let result: Result<Option<String>> = (|| {
        let mut stmt = conn
            .prepare("SELECT id, simhash FROM memories WHERE deleted_at IS NULL")?;
        let rows: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows
            .into_iter()
            .map(|(id, h)| (id, crate::simhash::hamming64(hash, h as u64)))
            .filter(|(_, d)| *d <= 3)
            .min_by_key(|(_, d)| *d)
            .map(|(id, _)| id))
    })();
    match result {
        Ok(hit) => hit,
        Err(e) => {
            tracing::warn!(error = %e, "duplicate check skipped");
            None // dup check is best-effort: never blocks a save
        }
    }
}
```

Call it in `run()` after the DB opens and the body is known, **before** the markdown write. Extend `Output`:

```rust
#[derive(serde::Serialize)]
struct Output {
    id: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duplicate_of: Option<String>,
}
```

TTY path: when `duplicate_of` is set, emit a warning line through the save emitter in `src/output/` (owo-colors yellow, e.g. `warning: similar memory a1b2c3d4 exists — consider supersedes`). No `println!` — use the output module's existing writer.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --all-features -E 'binary(cli) or binary(store)'`
Expected: PASS. The SimHash near-dup threshold on short bodies can be touchy — if the two seeded bodies don't land within Hamming ≤ 3, lengthen the shared phrasing in the test (more overlapping tokens) rather than loosening the threshold.

- [ ] **Step 5: Commit**

```bash
git add src/store/memory_row.rs src/cli/save.rs src/output tests
git commit -m "feat(save): simhash persistence + near-duplicate warning with duplicate_of hint"
```

---

### Task 11: Prune rewired to usage signals

**Files:**
- Modify: `src/prune/low_value.rs` (signal-based detection)
- Modify: `src/cli/prune.rs` (report + apply low-value soft-deletes)
- Test: mirrors `tests/prune/low_value.rs` + integration in the prune CLI test file

- [ ] **Step 1: Write the failing test**

`tests/prune/low_value.rs` (rewrite to the new contract):

```rust
use comemory::prune::low_value::detect;

#[test]
fn flags_only_cold_unloved_low_quality_unreferenced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, access_count, last_accessed, simhash)
         VALUES
         -- cold, downvoted, q2, unreferenced → flagged
         ('aaaa0001','a','note','d','f',2,1,'h1','b1','2025-01-01T00:00:00Z','2025-01-01T00:00:00Z','m/1.md',0,'2025-01-01T00:00:00Z',1),
         -- same but quality 4 → survives
         ('aaaa0002','b','note','d','f',4,1,'h2','b2','2025-01-01T00:00:00Z','2025-01-01T00:00:00Z','m/2.md',0,'2025-01-01T00:00:00Z',2),
         -- same but frequently accessed → survives
         ('aaaa0003','c','note','d','f',2,1,'h3','b3','2025-01-01T00:00:00Z','2025-01-01T00:00:00Z','m/3.md',50,'2026-06-01T00:00:00Z',3),
         -- same but referenced by an edge → survives
         ('aaaa0004','e','note','d','f',2,1,'h4','b4','2025-01-01T00:00:00Z','2025-01-01T00:00:00Z','m/4.md',0,'2025-01-01T00:00:00Z',4);
         INSERT INTO feedback(memory_id, used_count, irrelevant_count) VALUES ('aaaa0001', 0, 2);
         INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0009','memory','aaaa0004','derived_from','2026-01-01T00:00:00Z');",
    ).expect("seed");
    let cfg = comemory::config::Config::defaults();
    let flagged = detect(&conn, &cfg).expect("detect");
    assert_eq!(flagged, vec!["aaaa0001".to_string()]);
}

#[test]
fn superseded_and_untouched_since_is_flagged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, access_count, last_accessed, simhash)
         VALUES
         ('aaaa0001','old','note','d','f',4,1,'h1','old way','2025-01-01T00:00:00Z','2025-01-01T00:00:00Z','m/1.md',3,'2025-06-01T00:00:00Z',1),
         ('aaaa0002','new','note','d','f',4,1,'h2','new way','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','m/2.md',0,'2026-01-01T00:00:00Z',2);
         INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0002','memory','aaaa0001','supersedes','2026-01-01T00:00:00Z');",
    ).expect("seed");
    let cfg = comemory::config::Config::defaults();
    // aaaa0001: superseded by live aaaa0002, last_accessed (2025-06) < edge created (2026-01) → flagged
    let flagged = detect(&conn, &cfg).expect("detect");
    assert!(flagged.contains(&"aaaa0001".to_string()));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --all-features -E 'binary(prune)'`
Expected: compile FAILURE — `detect` signature differs.

- [ ] **Step 3: Implement**

`src/prune/low_value.rs` — new signature `pub fn detect(conn: &Connection, cfg: &Config) -> Result<Vec<String>>`:

```rust
//! Low-value memory detection driven by the same signals the rank
//! pipeline uses: activation, Beta feedback, quality, graph degree —
//! plus an independent superseded-and-forgotten rule.

use rusqlite::Connection;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::score;

/// Memories matching ALL of: activation below floor, feedback at/below
/// ceiling, quality ≤ below_quality, zero incoming edges — plus any
/// memory superseded by a live one and never accessed since.
pub fn detect(conn: &Connection, cfg: &Config) -> Result<Vec<String>> {
    let now = OffsetDateTime::now_utc();
    let mut flagged = signal_rule(conn, cfg, now)?;
    for id in superseded_rule(conn)? {
        if !flagged.contains(&id) {
            flagged.push(id);
        }
    }
    flagged.sort();
    Ok(flagged)
}

fn signal_rule(conn: &Connection, cfg: &Config, now: OffsetDateTime) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.quality, m.access_count, COALESCE(m.last_accessed, m.created_at),
                COALESCE(f.used_count, 0), COALESCE(f.irrelevant_count, 0)
           FROM memories m
           LEFT JOIN feedback f ON f.memory_id = m.id
          WHERE m.deleted_at IS NULL
            AND m.quality <= ?1
            AND NOT EXISTS (SELECT 1 FROM edges e
                             WHERE e.dst_kind = 'memory' AND e.dst_id = m.id)",
    )?;
    let rows: Vec<(String, u8, i64, String, i64, i64)> = stmt
        .query_map([cfg.prune.below_quality], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut out = Vec::new();
    for (id, _q, access, last, used, irrelevant) in rows {
        let days = days_since(&last, now);
        let act = score::activation(access.max(0) as u64, days, cfg.rank.decay);
        let beta = score::beta_feedback(used.max(0) as u64, irrelevant.max(0) as u64);
        if act < cfg.prune.min_activation && beta <= cfg.prune.min_feedback {
            out.push(id);
        }
    }
    Ok(out)
}

fn superseded_rule(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT old.id FROM memories old
           JOIN edges e ON e.rel = 'supersedes'
                       AND e.dst_kind = 'memory' AND e.dst_id = old.id
           JOIN memories newer ON newer.id = e.src_id AND newer.deleted_at IS NULL
          WHERE old.deleted_at IS NULL
            AND COALESCE(old.last_accessed, old.created_at) < e.created_at",
    )?;
    let ids = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(ids)
}

fn days_since(rfc3339: &str, now: OffsetDateTime) -> f64 {
    match OffsetDateTime::parse(rfc3339, &Rfc3339) {
        Ok(then) => ((now - then).whole_seconds() as f64 / 86_400.0).max(0.0),
        Err(_) => 0.0,
    }
}
```

Note the spec change: condition is `quality ≤ below_quality` (default 2, now inclusive) — update the default name/semantics consistently. The actual `PruneConfig` field names may differ from `below_quality` (defaults live at `config/file.rs:146-150` as `low_value_default_below_quality` / `low_value_default_unused_since_days`) — read the struct and use the real field names; `unused_since_days` is intentionally dropped from detection (activation replaces calendar age), but keep the config field for back-compat until M2 removes it. `days_since` duplicates rerank's — extract the shared helper into `src/retrieval/score.rs` (or a small `src/time_utils.rs`) so `scripts/dup-check.sh` stays green: one definition, two callers.

`src/cli/prune.rs`:
- `Report` gains `low_value_memories: Vec<String>`.
- `scan` calls `low_value::detect(conn, &cfg)` (load config the same way other commands do: `cli::load_config(&paths)`).
- `apply` additionally soft-deletes flagged memories via the same path `cli/delete.rs` uses (markdown → `.trash/` + `deleted_at` stamp + FTS row removal). Reuse the existing delete helper — do not reimplement (extract it into `src/memory/` if it's currently inline in `delete.rs`).
- Update `output::prune::emit` to print the new section (`low_value : N` + indented ids).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --all-features -E 'binary(prune) or binary(cli)'`
Expected: PASS (update existing prune CLI tests for the new Report field).

- [ ] **Step 5: Commit**

```bash
git add src/prune src/cli/prune.rs src/output tests
git commit -m "feat(prune): low-value detection from activation/feedback/edges + superseded rule"
```

---

### Task 12: Ranking smoke corpus (integration floor for recall)

**Files:**
- Create: `tests/common/corpus.rs` (declare `pub mod corpus;` in `tests/common/mod.rs`)
- Create: `tests/cli_rank_smoke.rs`

- [ ] **Step 1: Write the corpus + the test (this whole task is test code)**

`tests/common/corpus.rs` — 20 realistic memories. Content rules: real engineering phrasing, varied kinds/tags, deliberate identifier mentions, two near-duplicates, one supersede pair. Define:

```rust
/// (kind, body, tags, quality)
pub const CORPUS: &[(&str, &str, &str, u8)] = &[
    ("bug", "Postgres connection pool exhausts under load spikes; raise max_connections to 50 and add pgbouncer in transaction mode", "database,postgres", 4),
    ("decision", "We store embeddings as little-endian f32 blobs in sqlite-vec vec0 tables; dims are baked into DDL at migration time", "sqlite,vectors", 5),
    ("bug", "VecDimMismatch fires when the Ollama embedder returns 768 dims but memory_vec expects 1024 — check COMEMORY_EMBED_HINT", "vectors,ollama", 4),
    ("convention", "All CLI subcommands accept --json and emit a single-line JSON envelope on stdout; exit codes follow sysexits.h", "cli,output", 5),
    ("discovery", "FTS5 bm25() returns negative scores — lower is better; ORDER BY score ASC, not DESC", "sqlite,fts5", 4),
    ("pattern", "Use tracing::warn for best-effort failures that must not break the read path, e.g. access tracking updates", "errors,tracing", 3),
    ("bug", "git2 vendored-libgitgit2 build breaks on alpine without cmake; pin builder image to debian-slim", "ci,git", 3),
    ("decision", "Tests live strictly in tests/ mirroring src/ 1:1; pub(crate) items get promoted to pub when integration tests need them", "testing,conventions", 5),
    ("note", "cargo nextest profile serializes the embedder test group to avoid model download races", "testing,nextest", 3),
    ("discovery", "ast-grep pattern '$A.unwrap()' finds unwraps; pair with scripts/no-bypass-check.sh allowlist for tests/", "ast-grep,lint", 4),
    ("convention", "Conventional Commits with scope, e.g. feat(retrieval): …; release tags are v*", "git,conventions", 4),
    ("bug", "OAuth refresh race: two concurrent refreshes invalidate each other's tokens; serialize via per-user mutex", "auth,oauth", 5),
    ("pattern", "RRF fusion with k=60 over FTS5 + vec0 KNN lists; candidates capped at 50 before rerank", "retrieval,ranking", 4),
    ("note", "Homebrew tap publishes via cargo-dist on v* tags; PRs only get a dry-run plan", "release,homebrew", 3),
    ("decision", "Markdown files under ~/.comemory/memories are the source of truth; comemory rebuild reconstructs the DB", "architecture,storage", 5),
    ("bug", "Long camelCase identifiers like VecDimMismatch were unfindable before the identifier tokenizer split subtokens", "search,fts5", 4),
    ("discovery", "SQLite ALTER TABLE ADD COLUMN cannot default to another column; backfill with a follow-up UPDATE", "sqlite,migrations", 4),
    ("pattern", "Atomic file writes: stage to .{id}.tmp then fs::rename; remove the tmp on any failure", "io,reliability", 4),
    ("note", "owo-colors handles NO_COLOR automatically; never branch on tty manually", "output,tty", 2),
    ("convention", "Doc comments on every public item; rustfmt 100-col, 4-space indent", "style,docs", 4),
];

/// (query, expected substring of the top-3 bodies)
pub const SMOKE_QUERIES: &[(&str, &str)] = &[
    ("postgres pool exhausted", "pgbouncer"),
    ("VecDimMismatch", "768 dims"),
    ("vec dim mismatch", "768 dims"),
    ("bm25 negative score", "lower is better"),
    ("oauth token race", "per-user mutex"),
    ("rrf fusion constant", "k=60"),
    ("rebuild database from markdown", "source of truth"),
    ("camelcase identifier search", "identifier tokenizer"),
    ("alter table default backfill", "follow-up UPDATE"),
    ("atomic write rename", "fs::rename"),
];
```

`tests/cli_rank_smoke.rs`:

```rust
mod common;

use assert_cmd::Command;
use common::corpus::{CORPUS, SMOKE_QUERIES};

#[test]
fn smoke_queries_hit_expected_memory_in_top3() {
    let sandbox = common::runner::Sandbox::new();
    let data_dir = sandbox.data_dir();

    for (kind, body, tags, quality) in CORPUS {
        Command::cargo_bin("comemory")
            .expect("binary")
            .env("COMEMORY_DATA_DIR", &data_dir)
            .args(["--json", "save", body, "--kind", kind, "--tags", tags,
                   "--quality", &quality.to_string()])
            .assert()
            .success();
    }

    let mut failures = Vec::new();
    for (query, expected_fragment) in SMOKE_QUERIES {
        let out = Command::cargo_bin("comemory")
            .expect("binary")
            .env("COMEMORY_DATA_DIR", &data_dir)
            .args(["--json", "search", query, "--k", "3"])
            .assert()
            .success();
        let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).expect("json");
        let top3_ids: Vec<String> = v["hits"]
            .as_array()
            .expect("hits array")
            .iter()
            .map(|h| h["memory_id"].as_str().expect("id").to_string())
            .collect();
        // resolve bodies via list/search output: fetch each memory body
        let mut hit = false;
        for id in &top3_ids {
            let body_out = Command::cargo_bin("comemory")
                .expect("binary")
                .env("COMEMORY_DATA_DIR", &data_dir)
                .args(["--json", "list"])
                .assert()
                .success();
            let list: serde_json::Value =
                serde_json::from_slice(&body_out.get_output().stdout).expect("json");
            // find this id in the list payload and check the body fragment
            if list_contains_body(&list, id, expected_fragment) {
                hit = true;
                break;
            }
        }
        if !hit {
            failures.push(format!("query '{query}' missed '{expected_fragment}' in top-3"));
        }
    }
    assert!(failures.is_empty(), "recall@3 failures:\n{}", failures.join("\n"));
}

fn list_contains_body(list: &serde_json::Value, id: &str, fragment: &str) -> bool {
    // adapt to the actual `comemory list --json` shape during implementation
    list.as_array()
        .or_else(|| list.get("memories").and_then(|m| m.as_array()))
        .map(|arr| {
            arr.iter().any(|m| {
                m["id"].as_str() == Some(id)
                    && m["body"].as_str().is_some_and(|b| b.contains(fragment))
            })
        })
        .unwrap_or(false)
}
```

(If `list --json` doesn't include bodies, read the markdown file from `data_dir/memories/` by id-prefix instead — same assertion, filesystem lookup. Decide at implementation time based on the actual list payload; the markdown fallback always works because filenames are `{id}-{slug}.md`.)

- [ ] **Step 2: Add the two remaining spec-required integration tests** (same file, same sandbox helpers)

```rust
#[test]
fn irrelevant_feedback_reorders_results() {
    // seed two memories that both match the query; downvote the leader 3×;
    // rerun the query and assert the order flipped.
    let sandbox = common::runner::Sandbox::new();
    let data_dir = sandbox.data_dir();
    for body in [
        "sqlite busy timeout fix for the connection pool",
        "sqlite busy timeout workaround for pool checkout",
    ] {
        Command::cargo_bin("comemory")
            .expect("binary")
            .env("COMEMORY_DATA_DIR", &data_dir)
            .args(["--json", "save", body, "--kind", "bug"])
            .assert()
            .success();
    }
    let first = top_ids(&data_dir, "sqlite busy timeout");
    for _ in 0..3 {
        Command::cargo_bin("comemory")
            .expect("binary")
            .env("COMEMORY_DATA_DIR", &data_dir)
            .args(["feedback", "q1", "--irrelevant", &first[0]])
            .assert()
            .success();
    }
    let second = top_ids(&data_dir, "sqlite busy timeout");
    assert_ne!(first[0], second[0], "3× irrelevant must demote the leader");
}

#[test]
fn rebuild_preserves_search_results() {
    // save corpus → record top-3 for two queries → comemory rebuild →
    // same top-3 (access/feedback stats reset is accepted; with no feedback
    // recorded, ranking must be identical).
    let sandbox = common::runner::Sandbox::new();
    let data_dir = sandbox.data_dir();
    for (kind, body, tags, quality) in &CORPUS[..6] {
        Command::cargo_bin("comemory")
            .expect("binary")
            .env("COMEMORY_DATA_DIR", &data_dir)
            .args(["--json", "save", body, "--kind", kind, "--tags", tags,
                   "--quality", &quality.to_string()])
            .assert()
            .success();
    }
    let before: Vec<Vec<String>> =
        ["postgres pool", "vec dim"].iter().map(|q| top_ids(&data_dir, q)).collect();
    Command::cargo_bin("comemory")
        .expect("binary")
        .env("COMEMORY_DATA_DIR", &data_dir)
        .args(["rebuild"])
        .assert()
        .success();
    let after: Vec<Vec<String>> =
        ["postgres pool", "vec dim"].iter().map(|q| top_ids(&data_dir, q)).collect();
    assert_eq!(before, after, "rebuild must reproduce ranking from markdown");
}

fn top_ids(data_dir: &std::path::Path, query: &str) -> Vec<String> {
    let out = Command::cargo_bin("comemory")
        .expect("binary")
        .env("COMEMORY_DATA_DIR", data_dir)
        .args(["--json", "search", query, "--k", "3"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).expect("json");
    v["hits"].as_array().expect("hits")
        .iter()
        .map(|h| h["memory_id"].as_str().expect("id").to_string())
        .collect()
}
```

(Check `comemory feedback`'s real argument shape in `src/cli/feedback.rs:29-40` — positional `query_id` then `--irrelevant <ids>`; adjust args if the implementation differs.)

- [ ] **Step 3: Run the tests**

Run: `cargo nextest run --all-features -E 'binary(cli_rank_smoke)'`
Expected: PASS. If any individual query misses, this is a real ranking deficiency: first verify the expectation is fair (the expected memory really is the best answer), then fix ranking constants (BM25 column weights, boost mappings) — never delete the query to make the suite green. Document any constant change in the commit message.

- [ ] **Step 4: Commit**

```bash
git add tests/common/corpus.rs tests/common/mod.rs tests/cli_rank_smoke.rs
git commit -m "test: ranking smoke corpus, recall@3 floor, feedback reorder, rebuild parity"
```

---

### Task 13: Help text + docs touch-ups

**Files:**
- Modify: `src/cli/search.rs` EXAMPLES const (mention score_parts JSON + relaxed fallback)
- Modify: `src/cli/save.rs` EXAMPLES const (mention duplicate warning)
- Modify: `src/cli/prune.rs` EXAMPLES const (low-value section)
- Modify: `docs/cli-reference.md` (same three sections; describe `score_parts` fields)
- Modify: `CLAUDE.md` (Module Map: add `tokenizer` under store, `score/rerank/diversify/pipeline` under retrieval; Environment Variables table: add the five new vars; correct the spec's 0003→0004 reference if mentioned)
- Test: `tests/cli_help_examples.rs` exists — keep it green (it likely asserts EXAMPLES render in `--help`)

- [ ] **Step 1: Update EXAMPLES consts** — keep the existing format (they double as `after_help`). Example for search:

```text
Examples:
  comemory search "postgres pool exhausted"        # weighted BM25 + priors
  comemory search "VecDimMismatch"                 # identifier-aware matching
  comemory search "auth race" --json               # hits[].score_parts explains ranking
```

- [ ] **Step 2: Update `docs/cli-reference.md`** — add a `score_parts` field table (rrf, activation, feedback, quality, supersede, final_score) under the search section; document `duplicate_of` under save; document `low_value_memories` + the detection rule under prune; document the five new env vars in the env table.

- [ ] **Step 3: Update `CLAUDE.md`** module map + env table (same content as cli-reference env rows).

- [ ] **Step 4: Run help-examples test + commit**

Run: `cargo nextest run --all-features -E 'binary(cli_help_examples)'`
Expected: PASS.

```bash
git add src/cli docs/cli-reference.md CLAUDE.md
git commit -m "docs: score_parts, duplicate_of, low-value prune, rank env vars"
```

---

### Task 14: Full gate + final verification

- [ ] **Step 1: Umbrella gate**

Run: `bash scripts/check-all.sh`
Expected: exit 0 — fmt, type-check, clippy -D warnings, test placement, no-bypass, module size (every touched file ≤500 lines — split if `ffi.rs` or `rerank.rs` crept over), tests mirror, typos.

- [ ] **Step 2: Full test suite**

Run: `cargo nextest run --all-features`
Expected: all green.

- [ ] **Step 3: QA extras**

Run: `just qa` (check-all + cargo-deny + dup-check)
Expected: exit 0. New `libsqlite3-sys` direct dep must pass `cargo deny check` (it's already a transitive dep, so licensing is pre-cleared).

- [ ] **Step 4: Binary size check (project cap: 10 MB)**

Run: `cargo install --path . && ls -lh ~/.cargo/bin/comemory`
Expected: ≤ 10 MB (M1 adds no heavy deps; expect ~9 MB unchanged).

- [ ] **Step 5: Manual smoke (real binary)**

```bash
export COMEMORY_DATA_DIR=$(mktemp -d)
comemory save "VecDimMismatch fires when embedder dims disagree with vec0 DDL" --kind bug --tags vectors
comemory search "dim mismatch" --json   # → hit with score_parts
comemory save "VecDimMismatch fires when the embedder dims disagree with the vec0 DDL" --kind bug  # → duplicate warning
comemory doctor
```

Expected: search finds the memory via subtoken match; second save prints the duplicate hint; doctor green.

- [ ] **Step 6: Final commit (if any stragglers) — work complete**

---

## Self-review notes (already applied)

- Spec said migration `0003_v3_rank.sql` / version 3 → reality is `0004_v4_rank.sql` / version 4 (0003 already shipped as stats tables). Plan and spec header note this.
- Spec's `quality ≤ 2` prune condition maps to the existing `below_quality` knob with inclusive comparison — Task 11 notes the semantic alignment.
- `Reranked.body`/`simhash` carried through stages so diversify needs no second DB pass.
- `tests/` is a separate crate: everything tests touch must be `pub` (not `pub(crate)`) — Tasks 1, 3, 5, 6, 7 call this out.
- Tokenizer registration ordering (before migrations) is load-bearing: bundled SQLite 3.46 resolves `tokenize=` eagerly at prepare. Task 2 wires it before `migrate::run`, Task 3's migration is the first DDL that references it.
