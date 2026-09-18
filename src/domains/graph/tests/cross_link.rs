#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/domains/graph/cross_link.rs` — backtick-fenced
//! `<repo>:<path>[:<symbol>]` reference extraction from memory bodies.

use comemory::domains::graph::cross_link::extract_refs;

#[test]
fn extracts_file_and_symbol_refs() {
    let body = "See qwick-backend:src/db.rs:run_migration for the call; \
        also touches qwick-backend:src/util.rs.";
    let r = extract_refs(body);
    assert!(
        r.files.contains(&"qwick-backend:src/db.rs".to_string()),
        "expected qwick-backend:src/db.rs in files, got {:?}",
        r.files,
    );
    assert!(
        r.files.contains(&"qwick-backend:src/util.rs".to_string()),
        "expected qwick-backend:src/util.rs in files, got {:?}",
        r.files,
    );
    assert!(
        r.symbols
            .contains(&"qwick-backend:src/db.rs:run_migration".to_string()),
        "expected qwick-backend:src/db.rs:run_migration in symbols, got {:?}",
        r.symbols,
    );
}

#[test]
fn symbol_match_still_yields_file_ref() {
    // A symbol mention must also register the parent file so a memory that
    // names only `<repo>:<path>:<sym>` still gets a ReferencesFile edge.
    let r = extract_refs("touches qwick-frontend:src/app.ts:render");
    assert_eq!(
        r.files,
        vec!["qwick-frontend:src/app.ts".to_string()],
        "file ref should be derived from symbol mention",
    );
    assert_eq!(
        r.symbols,
        vec!["qwick-frontend:src/app.ts:render".to_string()],
    );
}

#[test]
fn duplicate_mentions_are_collapsed() {
    let body = "First qwick-backend:src/db.rs and again qwick-backend:src/db.rs.";
    let r = extract_refs(body);
    assert_eq!(
        r.files.len(),
        1,
        "duplicate file mention must collapse, got {:?}",
        r.files,
    );
    assert!(
        r.symbols.is_empty(),
        "no symbol expected, got {:?}",
        r.symbols
    );
}

#[test]
fn body_without_refs_returns_empty() {
    let r = extract_refs("Just a plain memory body with no code references.");
    assert!(r.files.is_empty());
    assert!(r.symbols.is_empty());
}

#[test]
fn requires_extension_to_match() {
    // Without an extension on the path, the regex must not match — guards
    // against false positives like `org:project`.
    let r = extract_refs("see qwick-backend:src/db for the call");
    assert!(r.files.is_empty(), "no extension should mean no match");
    assert!(r.symbols.is_empty());
}

#[test]
fn ignores_url_like_matches() {
    // Prose memories often include URLs to source files. The regex would
    // otherwise capture `https:` as a repo and `//github.com/.../bar.rs` as
    // a path; the scp-style git URL `git@host:foo/bar.rs:fn` would yield a
    // symbol ref. The URL filter (post-extraction) MUST drop both shapes.
    let body = "see https://github.com/foo/bar.rs and git@github.com:foo/bar.rs:fn for details";
    let r = extract_refs(body);
    assert!(
        r.files.is_empty(),
        "URL-like matches must not produce file refs, got {:?}",
        r.files,
    );
    assert!(
        r.symbols.is_empty(),
        "URL-like matches must not produce symbol refs, got {:?}",
        r.symbols,
    );
}

/// Mutation guard for the prefix-start offset at
/// `src/domains/graph/cross_link.rs:70` (`.map(|i| i + 1)`).
///
/// `i` is the index of the last whitespace before the match; `i + 1` is
/// the first byte of the token, so the URL-prefix slice starts AFTER the
/// separator. Here an `@` sits in its OWN whitespace-separated token right
/// before a clean ref:
///
/// ```text
/// @ qwick-backend:src/db.rs
/// ^ ^^ the space at index 1 is the last whitespace before the match
/// 0 2  the match begins at index 2; correct prefix = bytes[2..2] = ""
/// ```
///
/// With the correct `+ 1`, the prefix is empty → no `@` → the ref is KEPT.
/// The `+`→`-` mutant makes the prefix start at index 0, pulling the `@`
/// (and the space) into the prefix → the `@` URL-hallmark fires and the
/// ref is wrongly DROPPED. Asserting the ref survives kills `+`→`-`.
#[test]
fn at_sign_in_a_separate_token_does_not_taint_the_following_ref() {
    let r = extract_refs("@ qwick-backend:src/db.rs");
    assert_eq!(
        r.files,
        vec!["qwick-backend:src/db.rs".to_string()],
        "an `@` in its own token precedes the ref by whitespace; the prefix \
         must start after that whitespace, leaving the ref intact",
    );
    assert!(
        r.symbols.is_empty(),
        "no symbol expected, got {:?}",
        r.symbols
    );
}

/// Mutation guard for the URL-window equality at
/// `src/domains/graph/cross_link.rs:73` (`prefix.windows(3).any(|w| w == b"://")`).
///
/// The check keeps a ref unless some 3-byte window of its non-whitespace
/// prefix equals `://`. Here a ref is glued to a three-dot lead-in inside
/// one contiguous run:
///
/// ```text
/// see ...qwick-backend:src/db.rs ok
///     ^^^ prefix = "..." (3 bytes, no `://`, no `@`)
/// ```
///
/// The correct `== b"://"` finds no `://` window → the ref is KEPT. The
/// `==`→`!=` mutant turns the test into "any window that ISN'T `://`",
/// which `...` trivially satisfies → the ref is wrongly DROPPED. Asserting
/// the ref survives (its prefix is a non-URL run of length ≥ 3) kills
/// `==`→`!=`.
#[test]
fn non_url_three_byte_prefix_keeps_the_ref() {
    let r = extract_refs("see ...qwick-backend:src/db.rs ok");
    assert_eq!(
        r.files,
        vec!["qwick-backend:src/db.rs".to_string()],
        "a `...` prefix is not a URL hallmark; the ref must be kept",
    );
    assert!(
        r.symbols.is_empty(),
        "no symbol expected, got {:?}",
        r.symbols
    );
}

/// Issue #153: a `file:` URL — SQLite/libSQL's single-slash scheme — is how
/// prose records a database path. It matched `<repo>:<path>` with the
/// scheme as the repo (`repo = "file"`, `path = "/tmp/check.db"`) and slipped
/// past the `://`/`@` guard, minting a `references_file` edge to a
/// pseudo-repo that no store can ever resolve. An absolute or dot-relative
/// captured path is never a repo-relative citation, so all three shapes
/// must yield nothing.
#[test]
fn scheme_prefixed_path_expressions_are_not_refs() {
    for body in [
        r#"Run the gate with TURSO_DATABASE_URL="file:/tmp/check.db" so knip can load drizzle.config.ts."#,
        "DATABASE_URL=file:../../packages/database/.data/local.db for the local build",
        "point libsql at sqlite:./data/local.db for the smoke run",
        "a triple-slash file:///var/lib/app/state.db is a URL too",
    ] {
        let r = extract_refs(body);
        assert!(
            r.files.is_empty(),
            "a scheme-prefixed path must not become a file ref: {body:?} → {:?}",
            r.files,
        );
        assert!(
            r.symbols.is_empty(),
            "a scheme-prefixed path must not become a symbol ref: {body:?} → {:?}",
            r.symbols,
        );
    }
}

/// The guard keys on the captured path's shape, not on the word before the
/// colon: a real repo-relative citation in the same body as a `file:` URL
/// survives, symbol suffix included.
#[test]
fn a_real_citation_beside_a_file_url_is_kept() {
    let r =
        extract_refs("set DB=file:/tmp/check.db, then read qwick-backend:src/db.rs:run_migration");
    assert_eq!(r.files, vec!["qwick-backend:src/db.rs".to_string()]);
    assert_eq!(
        r.symbols,
        vec!["qwick-backend:src/db.rs:run_migration".to_string()],
    );
}
